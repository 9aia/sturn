//! sturn — evented coordination over the git object store.
//!
//! The WAL is the state; the tree is a projection; the bus is a
//! cache. Composition root: wires ports (wal/bus/ws/vcs/config/
//! service) to impls (git plumbing, redb, toml, systemd).

use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use sturn_bus::Bus;
use sturn_claim as claims;
use sturn_config::ConfigSource;
use sturn_core::{topic, Config, Event, Op, Verdict};
use sturn_service::Service;
use sturn_vcs::Vcs;
use sturn_wal::Wal;
use sturn_ws::Ws;

#[derive(Parser)]
#[command(
    name = "sturn",
    version,
    about = "Evented coordination over the git object store — agents murmurate, not merge"
)]
struct Cli {
    /// Repo dir (default: cwd).
    #[arg(long, global = true)]
    repo: Option<PathBuf>,
    /// Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/).
    #[arg(long, global = true)]
    config: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Wire a repo for sturn — writes sturn.toml if absent.
    Init,
    /// Workspace namespaces — virtual checkouts that cost a pointer.
    Ws {
        #[command(subcommand)]
        cmd: WsCmd,
    },
    /// Advisory lock on a path glob — published *before* edits.
    Claim {
        /// Path glob, e.g. "src/parser.rs" or "src/**".
        glob: String,
        /// Workspace claiming it.
        #[arg(long)]
        ws: String,
        /// Seconds the claim lives (auto-expiry, no zombie locks).
        #[arg(long, default_value_t = 3600)]
        ttl: u64,
        /// Take the claim even if it conflicts with a live one.
        #[arg(long)]
        force: bool,
    },
    /// Drop a claim.
    Release {
        glob: String,
        #[arg(long)]
        ws: String,
    },
    /// Append a write op — stdin or --file becomes an ODB blob.
    Write {
        /// Path inside the tree.
        path: String,
        #[arg(long)]
        ws: String,
        #[arg(long)]
        file: Option<PathBuf>,
        /// Why this edit exists — attached to the op.
        #[arg(long)]
        intent: Option<String>,
    },
    /// Record why work exists — provenance as data.
    Intent {
        text: String,
        #[arg(long)]
        ws: String,
    },
    /// Replay the WAL — ops in order, JSONL.
    Log {
        /// One workspace, or all when omitted.
        #[arg(long)]
        ws: Option<String>,
    },
    /// Fold a ws's ops into a real commit on the branch.
    Snap {
        ws: String,
        #[arg(short, long)]
        message: String,
        /// Branch to update (default: repo.branch / main).
        #[arg(long)]
        branch: Option<String>,
    },
    /// Approve/reject an event — continuous review.
    Review {
        /// Event id (blob sha) to judge.
        event: String,
        /// approve | reject.
        verdict: String,
        #[arg(long)]
        ws: String,
    },
    /// Push heads + refs/sturn/* — coordination rides the remote.
    Push {
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// WAL→topic fan-out loop. The bus is a cache — ops still sync
    /// through git when it's down.
    Daemon,
    /// Stream a topic live — ops|claims|review|presence, JSONL.
    Sub {
        /// Topic suffix under `repo.<id>.`.
        topic: String,
    },
    /// Live claims across every workspace.
    Claims,
    /// MCP stdio server — the 9-tool agent surface on JSON-RPC.
    Mcp,
    /// Hydrate a ws to tmpfs and run a command inside — compilers
    /// need POSIX; agents don't. Exits with the command's status.
    Exec {
        /// Workspace to hydrate.
        #[arg(long)]
        ws: String,
        /// Keep the hydrated dir after exit.
        #[arg(long)]
        keep: bool,
        /// Command + args, after `--`.
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },
    /// Deploy the daemon — systemd user unit + env skeleton.
    Deploy {
        #[command(subcommand)]
        cmd: DeployCmd,
    },
}

#[derive(Subcommand)]
enum WsCmd {
    /// Create a workspace — zero materialization, just a ref.
    New {
        name: String,
        /// Base commit/branch (default: HEAD).
        #[arg(long)]
        base: Option<String>,
    },
    /// List workspaces.
    Ls,
    /// Read a file from a ws tree — no checkout.
    Read { name: String, path: String },
    /// Materialize a ws tree into a dir — tip + uncommitted ops.
    /// --path filters to a subset (repeatable); empty = whole tree.
    Hydrate {
        name: String,
        dir: PathBuf,
        #[arg(long = "path")]
        paths: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DeployCmd {
    /// Write + enable the systemd user unit for `sturn daemon`.
    Service,
    /// Seed ~/.config/sturn/env (0600, names only).
    Env,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("sturn: {e}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    // Config is optional — everything works flag-driven too.
    let cfg: Option<Config> =
        sturn_config_toml::TomlSource {
            path: cli.config.clone(),
        }
        .load()
        .ok();
    let repo = cli
        .repo
        .clone()
        .or_else(|| cfg.as_ref().map(|c| c.repo.dir.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_id = cfg
        .as_ref()
        .map(|c| c.repo.id())
        .unwrap_or_else(|| {
            repo.canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "repo".into())
        });
    let branch = cfg
        .as_ref()
        .map(|c| c.repo.branch.clone())
        .unwrap_or_else(|| "main".into());
    let actor = cfg.as_ref().map(|c| c.actor.name()).unwrap_or_else(|| {
        std::env::var("STURN_ACTOR")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "anon".into())
    });
    let wal = sturn_wal_git::GitWal {
        ref_prefix: cfg
            .as_ref()
            .map(|c| c.wal.ref_prefix.clone())
            .unwrap_or_else(|| "refs/sturn/wal".into()),
    };
    let ws_impl = sturn_ws_git::GitWs;
    let bus_path = cfg
        .as_ref()
        .map(|c| c.bus.store.clone())
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| ".".into())
                .join(".local/state/sturn/bus.redb")
        });
    let poll_ms = cfg.as_ref().map(|c| c.bus.poll_ms).unwrap_or(100);

    match &cli.cmd {
        Cmd::Init => {
            if !Path::new(".git").exists() && !repo.join(".git").exists() {
                return Err("not a git repo — run inside one".into());
            }
            if !Path::new("sturn.toml").exists() {
                std::fs::write(
                    "sturn.toml",
                    "# sturn — coordination over git refs\n[repo]\ndir = \".\"\n",
                )
                .map_err(|e| e.to_string())?;
                println!("wrote sturn.toml");
            }
            println!(
                "sturn ready — WAL: refs/sturn/wal/*, ws: refs/sturn/ws/*"
            );
            Ok(())
        }
        Cmd::Ws { cmd } => match cmd {
            WsCmd::New { name, base } => {
                let base = base.clone().unwrap_or_else(|| "HEAD".into());
                ws_impl.create(&repo, name, &base).map_err(|e| e.to_string())?;
                println!("ws {name} → {base}");
                Ok(())
            }
            WsCmd::Ls => {
                // Union: ws namespaces (pointer-only) + WAL refs.
                let mut names = ws_impl.workspaces(&repo).unwrap_or_default();
                names.extend(wal.workspaces(&repo).unwrap_or_default());
                names.sort();
                names.dedup();
                names.iter().for_each(|n| println!("{n}"));
                Ok(())
            }
            WsCmd::Read { name, path } => {
                let tip = ws_impl.tip(&repo, name).map_err(|e| e.to_string())?;
                let bytes = ws_impl
                    .read(&repo, &tip, path)
                    .map_err(|e| e.to_string())?;
                print!("{}", String::from_utf8_lossy(&bytes));
                Ok(())
            }
            WsCmd::Hydrate { name, dir, paths } => {
                hydrate_ws(&repo, &wal, &ws_impl, name, dir, paths)
                    .map_err(|e| e.to_string())?;
                println!("hydrated {name} → {}", dir.display());
                Ok(())
            }
        },
        Cmd::Claim {
            glob,
            ws,
            ttl,
            force,
        } => {
            let all = all_events(&wal, &repo)?;
            let now = now();
            let live = claims::active(&all, now);
            // Overlap within the same ws is fine — one workspace,
            // one writer.
            let hits: Vec<_> = claims::conflicts(&live, glob)
                .into_iter()
                .filter(|c| c.ws != *ws)
                .collect();
            if !hits.is_empty() && !force {
                for c in &hits {
                    eprintln!(
                        "conflict: {} claimed {} (ws {}, expires {})",
                        c.actor, c.glob, c.ws, c.expires
                    );
                }
                return Err("claim denied — --force to override".into());
            }
            append(&wal, &repo, ws, &actor, Op::Claim {
                glob: glob.clone(),
                ttl_secs: *ttl,
            })?;
            println!("claimed {glob} (ws {ws}, ttl {ttl}s)");
            Ok(())
        }
        Cmd::Release { glob, ws } => {
            append(&wal, &repo, ws, &actor, Op::Release {
                glob: glob.clone(),
            })?;
            println!("released {glob}");
            Ok(())
        }
        Cmd::Claims => {
            let all = all_events(&wal, &repo)?;
            for c in claims::active(&all, now()) {
                println!(
                    "{} (ws {}, actor {}, expires {})",
                    c.glob, c.ws, c.actor, c.expires
                );
            }
            Ok(())
        }
        Cmd::Write {
            path,
            ws,
            file,
            intent,
        } => {
            let content = match file {
                Some(f) => std::fs::read(f).map_err(|e| e.to_string())?,
                None => {
                    let mut b = Vec::new();
                    std::io::stdin()
                        .read_to_end(&mut b)
                        .map_err(|e| e.to_string())?;
                    b
                }
            };
            let blob = sturn_git::store_blob(&repo, &content).map_err(|e| e.to_string())?;
            let id = append(
                &wal,
                &repo,
                ws,
                &actor,
                Op::Write {
                    path: path.clone(),
                    blob: blob.clone(),
                    intent: intent.clone(),
                },
            )?;
            println!("write {path} (blob {blob}, event {id})");
            Ok(())
        }
        Cmd::Intent { text, ws } => {
            append(&wal, &repo, ws, &actor, Op::Intent {
                text: text.clone(),
            })?;
            Ok(())
        }
        Cmd::Log { ws } => {
            let names = match ws {
                Some(w) => vec![w.clone()],
                None => wal.workspaces(&repo).map_err(|e| e.to_string())?,
            };
            for n in names {
                for ev in wal.events(&repo, &n).map_err(|e| e.to_string())? {
                    println!("{}", serde_json::to_string(&ev).map_err(|e| e.to_string())?);
                }
            }
            Ok(())
        }
        Cmd::Snap {
            ws,
            message,
            branch: b,
        } => {
            let commit = do_snap(
                &repo,
                &wal,
                &ws_impl,
                ws,
                message,
                b.as_deref().unwrap_or(&branch),
                &actor,
            )
            .map_err(|e| e.to_string())?;
            println!("snap {commit} → {}", b.as_deref().unwrap_or(&branch));
            Ok(())
        }
        Cmd::Mcp => {
            let snap = |ws: &str, msg: &str, branch: &str| {
                do_snap(&repo, &wal, &ws_impl, ws, msg, branch, &actor)
            };
            sturn_mcp::serve(&sturn_mcp::Ctx {
                repo: &repo,
                repo_id: &repo_id,
                wal: &wal,
                wsi: &ws_impl,
                actor: &actor,
                branch: &branch,
                snap: &snap,
            })
            .map_err(|e| e.to_string())
        }
        Cmd::Exec { ws, keep, cmd } => {
            let dir =
                std::env::temp_dir().join(format!("sturn-exec-{}", std::process::id()));
            hydrate_ws(&repo, &wal, &ws_impl, ws, &dir, &[])?;
            let status = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .current_dir(&dir)
                .status()
                .map_err(|e| e.to_string())?;
            if !*keep {
                let _ = std::fs::remove_dir_all(&dir);
            } else {
                eprintln!("kept {}", dir.display());
            }
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            Ok(())
        }
        Cmd::Review {
            event,
            verdict,
            ws,
        } => {
            let v = match verdict.as_str() {
                "approve" => Verdict::Approve,
                "reject" => Verdict::Reject,
                _ => return Err("verdict: approve|reject".into()),
            };
            append(&wal, &repo, ws, &actor, Op::Review {
                event: event.clone(),
                verdict: v,
            })?;
            println!("{verdict} {event}");
            Ok(())
        }
        Cmd::Push { remote } => {
            if sturn_git::GitVcs.push(&repo, remote) {
                println!("pushed heads + refs/sturn/* → {remote}");
                Ok(())
            } else {
                Err("push failed".into())
            }
        }
        Cmd::Daemon => {
            let bus =
                sturn_bus_redb::RedbBus::open(bus_path.clone()).map_err(|e| e.to_string())?;
            // Push side: unix socket next to the store. Subscribers
            // send one line "repo|topic", then get JSONL forever.
            let sock_path = bus_path.with_file_name("bus.sock");
            if let Some(d) = sock_path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::remove_file(&sock_path);
            let listener =
                UnixListener::bind(&sock_path).map_err(|e| e.to_string())?;
            let subs: Subs = Arc::new(Mutex::new(HashMap::new()));
            {
                let subs = subs.clone();
                std::thread::spawn(move || {
                    for conn in listener.incoming().flatten() {
                        let subs = subs.clone();
                        std::thread::spawn(move || handle_sub(conn, subs));
                    }
                });
            }
            let mut heads: HashMap<String, Option<String>> = HashMap::new();
            println!(
                "sturn daemon — fan-out {repo_id}.* (poll {poll_ms}ms, sub {})",
                sock_path.display()
            );
            loop {
                for ws in wal.workspaces(&repo).unwrap_or_default() {
                    let head = wal.head(&repo, &ws).unwrap_or(None);
                    let prev = heads.get(&ws).cloned().unwrap_or(None);
                    if head != prev {
                        // Only the delta — refs moved, walk the range.
                        for ev in wal
                            .events_since(&repo, &ws, prev.as_deref())
                            .unwrap_or_default()
                        {
                            let payload =
                                Arc::new(serde_json::to_vec(&ev).unwrap_or_default());
                            let _ = bus.publish(&repo_id, topic::OPS, &payload);
                            fanout(&subs, &repo_id, topic::OPS, &payload);
                            let t = match &ev.op {
                                Op::Claim { .. } | Op::Release { .. } => {
                                    Some(topic::CLAIMS)
                                }
                                Op::Review { .. } => Some(topic::REVIEW),
                                Op::Presence { .. } => Some(topic::PRESENCE),
                                _ => None,
                            };
                            if let Some(t) = t {
                                let _ = bus.publish(&repo_id, t, &payload);
                                fanout(&subs, &repo_id, t, &payload);
                            }
                        }
                        heads.insert(ws, head);
                    }
                }
                std::thread::sleep(Duration::from_millis(poll_ms));
            }
        }
        Cmd::Sub { topic } => {
            let sock_path = bus_path.with_file_name("bus.sock");
            let mut s = UnixStream::connect(&sock_path).map_err(|e| {
                format!("connect {}: {e} (is `sturn daemon` up?)", sock_path.display())
            })?;
            writeln!(s, "{repo_id}|{topic}").map_err(|e| e.to_string())?;
            let mut out = std::io::stdout().lock();
            std::io::copy(&mut s, &mut out).map_err(|e| e.to_string())?;
            Ok(())
        }
        Cmd::Deploy { cmd } => match cmd {
            DeployCmd::Service => {
                let svc = sturn_systemd::SystemdService {
                    unit: "sturn-daemon.service".into(),
                    bin: std::env::current_exe().map_err(|e| e.to_string())?,
                    env: std::env::var("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| ".".into())
                        .join(".config/sturn/env"),
                    config: cli.config.clone().map(PathBuf::from),
                };
                let p = svc.write().map_err(|e| e.to_string())?;
                svc.enable().map_err(|e| e.to_string())?;
                println!("enabled {}", p.display());
                Ok(())
            }
            DeployCmd::Env => {
                let dir = std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| ".".into())
                    .join(".config/sturn");
                let p = sturn_systemd::write_env_skeleton(&dir).map_err(|e| e.to_string())?;
                println!("env skeleton: {}", p.display());
                Ok(())
            }
        },
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn append(
    wal: &sturn_wal_git::GitWal,
    repo: &Path,
    ws: &str,
    actor: &str,
    op: Op,
) -> Result<String, String> {
    wal.append(repo, ws, &Event::new(ws, actor, op))
        .map_err(|e| e.to_string())
}

/// Fold a ws's pending ops (since last Snapshot) into a real commit
/// on `branch`, move the ws ref, append the Snapshot marker.
/// Shared by `sturn snap` and the MCP `snap` tool.
fn do_snap(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    ws: &str,
    message: &str,
    branch: &str,
    actor: &str,
) -> std::io::Result<String> {
    let base = wsi.tip(repo, ws)?;
    let events = wal.events(repo, ws)?;
    let since = sturn_core::since_last_snapshot(&events);
    let commit = sturn_git::snap_commit(repo, &base, &events[since..], message, branch)?;
    wsi.create(repo, ws, &commit)?;
    wal.append(
        repo,
        ws,
        &Event::new(
            ws,
            actor,
            Op::Snapshot {
                message: message.into(),
            },
        ),
    )?;
    Ok(commit)
}

/// Hydrate a ws into `dir`: tip commit via `git archive` (+ optional
/// path filter), then the WAL overlay since the last snapshot —
/// hydrated dirs see uncommitted ops, matching `ws_read`.
fn hydrate_ws(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    name: &str,
    dir: &Path,
    paths: &[String],
) -> Result<(), String> {
    let tip = wsi.tip(repo, name).map_err(|e| e.to_string())?;
    wsi.materialize(repo, &tip, dir, paths)
        .map_err(|e| e.to_string())?;
    let events = wal.events(repo, name).map_err(|e| e.to_string())?;
    let pending = &events[sturn_core::since_last_snapshot(&events)..];
    for (p, v) in sturn_core::overlay(pending, |p| {
        wsi.entry(repo, &tip, p).ok().flatten()
    }) {
        let f = dir.join(&p);
        match v {
            Some(sha) => {
                if let Some(d) = f.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(&f, wsi.blob(repo, &sha).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
            None => {
                let _ = std::fs::remove_file(&f);
            }
        }
    }
    Ok(())
}

/// Every workspace's events — claims are swarm-wide, so conflict
/// checks fold all WALs, not just the caller's.
fn all_events(wal: &sturn_wal_git::GitWal, repo: &Path) -> Result<Vec<Event>, String> {
    let mut all = Vec::new();
    for ws in wal.workspaces(repo).map_err(|e| e.to_string())? {
        all.extend(wal.events(repo, &ws).map_err(|e| e.to_string())?);
    }
    Ok(all)
}

/// Live subscribers, keyed `"<repo>|<topic>"` — the daemon's push
/// side. Dead senders get pruned on the next failed delivery.
type Subs = Arc<Mutex<HashMap<String, Vec<mpsc::Sender<Arc<Vec<u8>>>>>>>;

/// First line on the socket names the topic; then JSONL payloads
/// stream until the client hangs up.
fn handle_sub(mut conn: UnixStream, subs: Subs) {
    let mut key = String::new();
    let mut rdr = BufReader::new(match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    if rdr.read_line(&mut key).is_err() || key.trim().is_empty() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Arc<Vec<u8>>>();
    subs.lock()
        .unwrap()
        .entry(key.trim().to_string())
        .or_default()
        .push(tx);
    for msg in rx {
        if conn
            .write_all(&msg)
            .and_then(|_| conn.write_all(b"\n"))
            .is_err()
        {
            break;
        }
    }
}

/// Push a published payload to live subscribers of the topic.
fn fanout(subs: &Subs, repo: &str, topic: &str, payload: &Arc<Vec<u8>>) {
    let key = format!("{repo}|{topic}");
    let mut map = subs.lock().unwrap();
    if let Some(list) = map.get_mut(&key) {
        list.retain(|tx| tx.send(payload.clone()).is_ok());
    }
}
