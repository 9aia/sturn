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
    /// Append a rename op — intent survives where git loses it.
    Rename {
        from: String,
        to: String,
        #[arg(long)]
        ws: String,
    },
    /// Append a delete op.
    Delete {
        path: String,
        #[arg(long)]
        ws: String,
    },
    /// Record why work exists — provenance as data.
    Intent {
        text: String,
        #[arg(long)]
        ws: String,
    },
    /// Drop pre-snapshot WAL history (archived, not lost).
    Compact {
        /// One workspace, or all when omitted.
        #[arg(long)]
        ws: Option<String>,
    },
    /// Replay the WAL — ops in order, JSONL.
    Log {
        /// One workspace, or all when omitted.
        #[arg(long)]
        ws: Option<String>,
    },
    /// Fold ws ops into a real commit on the branch. Multiple ws =
    /// merged into ONE commit (ops concat; later ops win same-path).
    Snap {
        #[arg(required = true)]
        ws: Vec<String>,
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
    /// Announce presence — `leave` drops all the actor's claims.
    Join { ws: String },
    /// Leave a ws — sweeps the actor's claims on it.
    Leave { ws: String },
    /// Verify event signatures against allowed_signers.
    Verify {
        #[arg(long)]
        ws: Option<String>,
    },
    /// Live claims across every workspace.
    Claims,
    /// Bridge snapshot events to a bupha webhook — GitHub-shaped,
    /// HMAC-signed. Bupha runs CI on snaps with zero bupha changes.
    Bridge {
        /// bupha listener URL (e.g. http://127.0.0.1:9000/).
        #[arg(long)]
        to: String,
        /// Env var naming the webhook secret (HMAC-SHA256).
        #[arg(long, default_value = "BUPHA_WEBHOOK_SECRET")]
        secret_env: String,
    },
    /// MCP stdio server — the 9-tool agent surface on JSON-RPC.
    /// --ws makes it the agent's ws: presence ops emit on connect.
    Mcp {
        #[arg(long)]
        ws: Option<String>,
    },
    /// Hydrate a ws to tmpfs and run a command inside — compilers
    /// need POSIX; agents don't. Exits with the command's status.
    Exec {
        /// Workspace to hydrate.
        #[arg(long)]
        ws: String,
        /// Keep the hydrated dir after exit.
        #[arg(long)]
        keep: bool,
        /// Write filesystem edits back as ops on the ws.
        #[arg(long)]
        writeback: bool,
        /// Command + args, after `--`.
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },
    /// Mount a ws tree via FUSE — POSIX for tools/GUIs.
    /// --rw: edits flow back as Write/Delete/Rename ops.
    Mount {
        ws: String,
        dir: PathBuf,
        #[arg(long)]
        rw: bool,
    },
    /// Deploy the daemon — systemd user unit + env skeleton.
    Deploy {
        #[command(subcommand)]
        cmd: DeployCmd,
    },
    /// Teardown counterparts of `deploy`.
    Destroy {
        #[command(subcommand)]
        cmd: DestroySub,
    },
    /// Toggle the daemon unit without removing it — free the box
    /// when fan-out isn't needed, `enable` brings it back.
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
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
        /// After materializing, diff the dir back into ops now.
        #[arg(long)]
        writeback: bool,
    },
}

#[derive(Subcommand)]
enum DeployCmd {
    /// Write + enable the systemd user unit for `sturn daemon`.
    Service,
    /// Seed ~/.config/sturn/env (0600, names only).
    Env,
}

#[derive(Subcommand)]
enum DestroySub {
    /// Disable + remove the systemd user unit.
    Service,
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// Enable + start the daemon unit now and at login.
    Enable,
    /// Stop the daemon now; stays stopped across logins until
    /// `service enable` (the unit file is kept).
    Disable,
    /// Whether the daemon unit is enabled and running.
    Status,
    /// Bounce the daemon unit in place.
    Restart,
    /// Recent journal lines — `-f` follows the stream live.
    Logs {
        /// Follow the log stream (journalctl -f).
        #[arg(short = 'f', long = "f")]
        follow: bool,
    },
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
        sign_key: cfg
            .as_ref()
            .and_then(|c| c.signing.key.clone())
            .map(|k| {
                k.replace(
                    '~',
                    &std::env::var("HOME").unwrap_or_else(|_| ".".into()),
                )
            })
            .map(PathBuf::from),
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
            WsCmd::Hydrate {
                name,
                dir,
                paths,
                writeback,
            } => {
                hydrate_ws(&repo, &wal, &ws_impl, name, dir, paths)
                    .map_err(|e| e.to_string())?;
                if *writeback {
                    let n =
                        writeback_ops(&repo, &wal, &ws_impl, name, dir, &actor)?;
                    println!("writeback: {n} op(s)");
                }
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
        Cmd::Join { ws } => {
            append(&wal, &repo, ws, &actor, Op::Presence {
                status: "joined".into(),
            })?;
            println!("{actor} joined {ws}");
            Ok(())
        }
        Cmd::Leave { ws } => {
            append(&wal, &repo, ws, &actor, Op::Presence {
                status: "left".into(),
            })?;
            println!("{actor} left {ws} — claims swept");
            Ok(())
        }
        Cmd::Verify { ws } => {
            let signers = cfg
                .as_ref()
                .and_then(|c| c.signing.allowed_signers.clone())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| ".".into())
                        .join(".config/sturn/allowed_signers")
                });
            if !signers.exists() {
                return Err(format!(
                    "no allowed_signers at {} — can't verify",
                    signers.display()
                ));
            }
            let names = match ws {
                Some(w) => vec![w.clone()],
                None => wal.workspaces(&repo).map_err(|e| e.to_string())?,
            };
            let (mut ok, mut nosig, mut bad) = (0u32, 0u32, 0u32);
            for n in names {
                for ev in wal.events(&repo, &n).map_err(|e| e.to_string())? {
                    let Some(sig) = &ev.sig else {
                        nosig += 1;
                        continue;
                    };
                    let dir = std::env::temp_dir();
                    let data = dir.join(format!("sturn-vfy-{}", std::process::id()));
                    let sigf = dir.join(format!("sturn-vfy-{}.sig", std::process::id()));
                    std::fs::write(&data, ev.signing_bytes()).map_err(|e| e.to_string())?;
                    std::fs::write(&sigf, sig).map_err(|e| e.to_string())?;
                    let st = std::process::Command::new("ssh-keygen")
                        .args(["-Y", "verify", "-f"])
                        .arg(&signers)
                        .args(["-I", &ev.actor, "-n", "sturn-event", "-s"])
                        .arg(&sigf)
                        .stdin(std::process::Stdio::from(
                            std::fs::File::open(&data).map_err(|e| e.to_string())?,
                        ))
                        .stdout(std::process::Stdio::null())
                        .status();
                    let _ = std::fs::remove_file(&data);
                    let _ = std::fs::remove_file(&sigf);
                    match st.map(|s| s.success()) {
                        Ok(true) => ok += 1,
                        _ => {
                            bad += 1;
                            eprintln!("bad sig: {} ({})", ev.id, ev.actor);
                        }
                    }
                }
            }
            println!("verify: {ok} ok, {nosig} unsigned, {bad} bad");
            if bad > 0 {
                Err("bad signatures found".into())
            } else {
                Ok(())
            }
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
        Cmd::Rename { from, to, ws } => {
            append(&wal, &repo, ws, &actor, Op::Rename {
                from: from.clone(),
                to: to.clone(),
            })?;
            println!("rename {from} → {to}");
            Ok(())
        }
        Cmd::Delete { path, ws } => {
            append(&wal, &repo, ws, &actor, Op::Delete {
                path: path.clone(),
            })?;
            println!("delete {path}");
            Ok(())
        }
        Cmd::Intent { text, ws } => {
            append(&wal, &repo, ws, &actor, Op::Intent {
                text: text.clone(),
            })?;
            Ok(())
        }
        Cmd::Compact { ws } => {
            let names = match ws {
                Some(w) => vec![w.clone()],
                None => wal.workspaces(&repo).map_err(|e| e.to_string())?,
            };
            for n in names {
                let dropped = wal.compact(&repo, &n).map_err(|e| e.to_string())?;
                println!("{n}: dropped {dropped} pre-snapshot events");
            }
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
            let require = cfg.as_ref().map(|c| c.review.require).unwrap_or(false);
            let br = b.as_deref().unwrap_or(&branch);
            if ws.len() == 1 {
                let commit = do_snap(
                    &repo, &wal, &ws_impl, &ws[0], message, br, &actor, require,
                )
                .map_err(|e| e.to_string())?;
                println!("snap {commit} → {br}");
            } else {
                let commit = merge_snap(
                    &repo, &wal, &ws_impl, ws, message, br, &actor, require,
                )?;
                println!("snap {commit} → {br} ({} ws merged)", ws.len());
            }
            Ok(())
        }
        Cmd::Bridge { to, secret_env } => {
            let secret = std::env::var(secret_env)
                .map_err(|_| format!("${secret_env} not set"))?;
            run_bridge(&repo, &wal, &ws_impl, to, &secret, &repo_id, &branch, poll_ms)
        }
        Cmd::Mcp { ws: mws } => {
            let snap = |ws: &str, msg: &str, branch: &str| {
                do_snap(
                    &repo,
                    &wal,
                    &ws_impl,
                    ws,
                    msg,
                    branch,
                    &actor,
                    cfg.as_ref().map(|c| c.review.require).unwrap_or(false),
                )
            };
            sturn_mcp::serve(&sturn_mcp::Ctx {
                repo: &repo,
                repo_id: &repo_id,
                wal: &wal,
                wsi: &ws_impl,
                actor: &actor,
                branch: &branch,
                snap: &snap,
                ws: mws.as_deref(),
            })
            .map_err(|e| e.to_string())
        }
        Cmd::Exec { ws, keep, writeback, cmd } => {
            let dir =
                std::env::temp_dir().join(format!("sturn-exec-{}", std::process::id()));
            hydrate_ws(&repo, &wal, &ws_impl, ws, &dir, &[])?;
            let status = std::process::Command::new(&cmd[0])
                .args(&cmd[1..])
                .current_dir(&dir)
                .status()
                .map_err(|e| e.to_string())?;
            if *writeback {
                let n = writeback_ops(&repo, &wal, &ws_impl, ws, &dir, &actor)?;
                eprintln!("writeback: {n} op(s)");
            }
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
        // The poll loop is infinite by design — the driver is
        // excluded under coverage; `daemon_tick` (one iteration),
        // `handle_sub` and `fanout` carry the logic and are
        // unit-tested directly.
        #[cfg(not(coverage))]
        Cmd::Daemon => daemon(&repo, &cfg, &wal, &bus_path, poll_ms, &repo_id)
            .map_err(|e| e.to_string()),
        #[cfg(coverage)]
        Cmd::Daemon => Ok(()),
        Cmd::Sub { topic } => {
            let sock_path = bus_path.with_file_name("bus.sock");
            subscribe(&sock_path, &repo_id, topic)
        }
        Cmd::Mount { ws, dir, rw } => {
            let tip = ws_impl.tip(&repo, ws).map_err(|e| e.to_string())?;
            if *rw {
                sturn_fuse::SturnFs::new_rw(&repo, &tip, &ws_impl, &wal, ws, &actor)
                    .and_then(|fs| fs.mount(dir))
                    .map_err(|e| e.to_string())
            } else {
                sturn_fuse::SturnFs::new(&repo, &tip, &ws_impl)
                    .and_then(|fs| fs.mount(dir))
                    .map_err(|e| e.to_string())
            }
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
        Cmd::Destroy { cmd } => match cmd {
            DestroySub::Service => {
                let p = sturn_systemd::disable_service("sturn-daemon.service")
                    .map_err(|e| e.to_string())?;
                println!("removed {}", p.display());
                Ok(())
            }
        },
        Cmd::Service { cmd } => {
            // Toggle only — `deploy service` writes the unit, and the
            // file stays put through enable/disable cycles.
            let unit = "sturn-daemon.service";
            match cmd {
                ServiceCmd::Enable => {
                    sturn_systemd::enable_service(unit).map_err(|e| e.to_string())?;
                    println!("sturn-daemon enabled — running now and at login");
                }
                ServiceCmd::Disable => {
                    sturn_systemd::stop_service(unit).map_err(|e| e.to_string())?;
                    println!("sturn-daemon disabled — stopped, will not restart at login");
                }
                ServiceCmd::Status => {
                    let s = sturn_systemd::unit_status(unit).map_err(|e| e.to_string())?;
                    println!("{s}");
                }
                ServiceCmd::Restart => {
                    sturn_systemd::restart_service(unit).map_err(|e| e.to_string())?;
                    println!("sturn-daemon restarted");
                }
                ServiceCmd::Logs { follow } => {
                    let code = sturn_systemd::service_logs(unit, *follow)
                        .map_err(|e| e.to_string())?;
                    if code != 0 {
                        std::process::exit(code);
                    }
                }
            }
            Ok(())
        }
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
    require_review: bool,
) -> std::io::Result<String> {
    let base = wsi.tip(repo, ws)?;
    let events = wal.events(repo, ws)?;
    let since = sturn_core::since_last_snapshot(&events);
    if require_review {
        // Approvals are swarm-wide — any ws's WAL may hold them.
        let mut approved = std::collections::HashSet::new();
        for w in wal.workspaces(repo)? {
            for e in wal.events(repo, &w)? {
                if let Op::Review {
                    event,
                    verdict: Verdict::Approve,
                } = &e.op
                {
                    approved.insert(event.clone());
                }
            }
        }
        let unreviewed: Vec<&Event> = events[since..]
            .iter()
            .filter(|e| {
                matches!(
                    e.op,
                    Op::Write { .. } | Op::Rename { .. } | Op::Delete { .. }
                )
            })
            .filter(|e| !approved.contains(&e.id))
            .collect();
        if !unreviewed.is_empty() {
            let ids: Vec<&str> = unreviewed.iter().map(|e| e.id.as_str()).collect();
            return Err(std::io::Error::other(format!(
                "review gate: {} unreviewed op(s) — {}",
                unreviewed.len(),
                ids.join(", ")
            )));
        }
    }
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

/// POST a GitHub-shaped push webhook (HMAC-SHA256 signed) — bupha's
/// GitHubTrigger accepts it as an ordinary push. Pure std::net.
fn post_push(
    url: &str,
    secret: &str,
    repo_id: &str,
    branch: &str,
    sha: &str,
    sender: &str,
) -> Result<(), String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let body = format!(
        "{{\"ref\":\"refs/heads/{branch}\",\"after\":\"{sha}\",\
         \"repository\":{{\"full_name\":\"{repo_id}\"}},\"sender\":{{\"login\":\"{sender}\"}}}}"
    );
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(body.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    // minimal HTTP/1.1 POST over TcpStream
    let url = url.trim_end_matches('/');
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (hostport, path) = host.split_once('/').unwrap_or((host, ""));
    let mut s = std::net::TcpStream::connect(hostport).map_err(|e| e.to_string())?;
    let req = format!(
        "POST /{path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\n\
         X-GitHub-Event: push\r\nX-Hub-Signature-256: sha256={sig}\r\n\
         X-GitHub-Delivery: sturn-{sha}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).map_err(|e| e.to_string())?;
    if resp.starts_with("HTTP/1.1 2") || resp.starts_with("HTTP/1.0 2") {
        Ok(())
    } else {
        Err(format!("bridge POST {url}: {}", resp.lines().next().unwrap_or("?")))
    }
}

/// One bridge poll iteration — publish webhook posts for snapshot
/// events since each ws's last-seen head. Extracted so tests drive
/// it directly; the `loop` around it is coverage-excluded.
#[allow(clippy::too_many_arguments)]
fn bridge_tick(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    heads: &mut HashMap<String, Option<String>>,
    to: &str,
    secret: &str,
    repo_id: &str,
    branch: &str,
) {
    for ws in wal.workspaces(repo).unwrap_or_default() {
        let head = wal.head(repo, &ws).unwrap_or(None);
        let prev = heads.get(&ws).cloned().unwrap_or(None);
        if head == prev {
            continue;
        }
        for ev in wal
            .events_since(repo, &ws, prev.as_deref())
            .unwrap_or_default()
        {
            if let Op::Snapshot { .. } = ev.op {
                if let Ok(sha) = wsi.tip(repo, &ws) {
                    let _ = post_push(to, secret, repo_id, branch, &sha, &ev.actor);
                }
            }
        }
        heads.insert(ws, head);
    }
}

/// WAL→topic fan-out loop. Infinite by design — excluded under
/// coverage; everything it does per iteration lives in
/// `daemon_tick`, `handle_sub` and `fanout`, which are unit-tested.
#[cfg(not(coverage))]
fn daemon(
    repo: &Path,
    cfg: &Option<Config>,
    wal: &sturn_wal_git::GitWal,
    bus_path: &Path,
    poll_ms: u64,
    repo_id: &str,
) -> std::io::Result<()> {
    let bus = sturn_bus_redb::RedbBus::open(bus_path.to_path_buf())?;
    // Push side: unix socket next to the store. Subscribers
    // send one line "repo|topic", then get JSONL forever.
    let sock_path = bus_path.with_file_name("bus.sock");
    if let Some(d) = sock_path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path)?;
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
    // Multi-repo: this repo + any [bus] repos. Topics key
    // per repo id; heads key per (dir, ws).
    let mut dirs: Vec<PathBuf> = vec![repo.to_path_buf()];
    dirs.extend(cfg.as_ref().map(|c| c.bus.repos.clone()).unwrap_or_default());
    let mut heads: HashMap<(PathBuf, String), Option<String>> = HashMap::new();
    println!(
        "sturn daemon — fan-out {} repo(s) (poll {poll_ms}ms, sub {})",
        dirs.len(),
        sock_path.display()
    );
    loop {
        daemon_tick(wal, &dirs, repo, repo_id, &mut heads, &bus, &subs);
        std::thread::sleep(Duration::from_millis(poll_ms));
    }
}

/// Bridge driver: banner + the infinite poll loop. Excluded under
/// coverage (infinite by design); `bridge_tick` is unit-tested.
#[cfg(not(coverage))]
#[allow(clippy::too_many_arguments)]
fn run_bridge(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    to: &str,
    secret: &str,
    repo_id: &str,
    branch: &str,
    poll_ms: u64,
) -> Result<(), String> {
    println!("sturn bridge — {repo_id} snapshots → {to}");
    bridge(repo, wal, wsi, to, secret, repo_id, branch, poll_ms)
}

/// Coverage build stub — returns immediately so the arm dispatch is
/// covered without an infinite loop.
#[cfg(coverage)]
#[allow(clippy::too_many_arguments)]
fn run_bridge(
    _repo: &Path,
    _wal: &sturn_wal_git::GitWal,
    _wsi: &sturn_ws_git::GitWs,
    _to: &str,
    _secret: &str,
    _repo_id: &str,
    _branch: &str,
    _poll_ms: u64,
) -> Result<(), String> {
    Ok(())
}

/// The snapshot-event → bupha webhook loop. `!` return — runs until
/// killed; excluded under coverage for the same reason as `daemon`.
#[cfg(not(coverage))]
#[allow(clippy::too_many_arguments)]
fn bridge(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    to: &str,
    secret: &str,
    repo_id: &str,
    branch: &str,
    poll_ms: u64,
) -> ! {
    let mut heads: HashMap<String, Option<String>> = HashMap::new();
    loop {
        bridge_tick(repo, wal, wsi, &mut heads, to, secret, repo_id, branch);
        std::thread::sleep(Duration::from_millis(poll_ms));
    }
}

/// One WAL→topic fan-out iteration across `dirs`: for each ws whose
/// head moved, publish the delta to `ops` plus the op-kind topic and
/// push to live socket subscribers.
fn daemon_tick(
    wal: &sturn_wal_git::GitWal,
    dirs: &[PathBuf],
    repo: &Path,
    repo_id: &str,
    heads: &mut HashMap<(PathBuf, String), Option<String>>,
    bus: &sturn_bus_redb::RedbBus,
    subs: &Subs,
) {
    for dir in dirs {
        let rid = if *dir == repo {
            repo_id.to_string()
        } else {
            dir.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into())
        };
        for ws in wal.workspaces(dir).unwrap_or_default() {
            let head = wal.head(dir, &ws).unwrap_or(None);
            let key = (dir.clone(), ws.clone());
            let prev = heads.get(&key).cloned().unwrap_or(None);
            if head != prev {
                // Only the delta — refs moved, walk the range.
                for ev in wal
                    .events_since(dir, &ws, prev.as_deref())
                    .unwrap_or_default()
                {
                    let payload =
                        Arc::new(serde_json::to_vec(&ev).unwrap_or_default());
                    let _ = bus.publish(&rid, topic::OPS, &payload);
                    fanout(subs, &rid, topic::OPS, &payload);
                    let t = match &ev.op {
                        Op::Claim { .. } | Op::Release { .. } => {
                            Some(topic::CLAIMS)
                        }
                        Op::Review { .. } => Some(topic::REVIEW),
                        Op::Presence { .. } => Some(topic::PRESENCE),
                        _ => None,
                    };
                    if let Some(t) = t {
                        let _ = bus.publish(&rid, t, &payload);
                        fanout(subs, &rid, t, &payload);
                    }
                }
                heads.insert(key, head);
            }
        }
    }
}

/// `sturn sub` — connect to the daemon socket, name the topic, then
/// stream JSONL to stdout until the socket closes.
fn subscribe(sock_path: &Path, repo_id: &str, topic: &str) -> Result<(), String> {
    let mut s = UnixStream::connect(sock_path).map_err(|e| {
        format!("connect {}: {e} (is `sturn daemon` up?)", sock_path.display())
    })?;
    writeln!(s, "{repo_id}|{topic}").map_err(|e| e.to_string())?;
    let mut out = std::io::stdout().lock();
    std::io::copy(&mut s, &mut out).map_err(|e| e.to_string())?;
    Ok(())
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
    // try_clone only fails on a dead fd — defensive.
    let mut rdr = BufReader::new(match conn.try_clone() { Ok(c) => c, Err(_) => return });
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

/// Walk `dir`, diff against the ws view (tip + pending overlay),
/// append Write/Delete ops for the difference. Returns ops written.
fn writeback_ops(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    ws: &str,
    dir: &Path,
    actor: &str,
) -> Result<usize, String> {
    let tip = wsi.tip(repo, ws).map_err(|e| e.to_string())?;
    let events = wal.events(repo, ws).map_err(|e| e.to_string())?;
    let pending = &events[sturn_core::since_last_snapshot(&events)..];
    let ov = sturn_core::overlay(pending, |p| {
        wsi.entry(repo, &tip, p).ok().flatten()
    });
    // expected path → content bytes (overlay wins, else tip)
    let mut expected: HashMap<String, Vec<u8>> = HashMap::new();
    for p in wsi.list(repo, &tip).map_err(|e| e.to_string())? {
        if let Ok(b) = wsi.read(repo, &tip, &p) {
            expected.insert(p, b);
        }
    }
    for (p, v) in &ov {
        match v {
            Some(sha) => {
                if let Ok(b) = wsi.blob(repo, sha) {
                    expected.insert(p.clone(), b);
                }
            }
            None => {
                expected.remove(p);
            }
        }
    }
    // walk the hydrated dir
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut n = 0;
    while let Some(d) = stack.pop() {
        for ent in std::fs::read_dir(&d).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            let ft = ent.file_type().map_err(|e| e.to_string())?;
            if ft.is_dir() {
                stack.push(ent.path());
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let rel = ent
                .path()
                .strip_prefix(dir)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned();
            seen.insert(rel.clone());
            let bytes = std::fs::read(ent.path()).map_err(|e| e.to_string())?;
            let differs = expected.get(&rel).map(|e| *e != bytes).unwrap_or(true);
            if differs {
                let blob =
                    wsi.store(repo, &bytes).map_err(|e| e.to_string())?;
                wal.append(
                    repo,
                    ws,
                    &Event::new(
                        ws,
                        actor,
                        Op::Write {
                            path: rel,
                            blob,
                            intent: Some("writeback".into()),
                        },
                    ),
                )
                .map_err(|e| e.to_string())?;
                n += 1;
            }
        }
    }
    for p in expected.keys() {
        if !seen.contains(p) {
            wal.append(
                repo,
                ws,
                &Event::new(
                    ws,
                    actor,
                    Op::Delete {
                        path: p.clone(),
                    },
                ),
            )
            .map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    Ok(n)
}

/// Merge several ws's pending ops into ONE commit on `branch`.
/// Later ops win on same-path collisions — claims should have
/// prevented that upstream.
fn merge_snap(
    repo: &Path,
    wal: &sturn_wal_git::GitWal,
    wsi: &sturn_ws_git::GitWs,
    wss: &[String],
    message: &str,
    branch: &str,
    actor: &str,
    require_review: bool,
) -> Result<String, String> {
    let mut pending: Vec<Event> = Vec::new();
    for ws in wss {
        let events = wal.events(repo, ws).map_err(|e| e.to_string())?;
        let since = sturn_core::since_last_snapshot(&events);
        pending.extend(events[since..].iter().cloned());
    }
    if require_review {
        let mut approved = std::collections::HashSet::new();
        for w in wal.workspaces(repo).map_err(|e| e.to_string())? {
            for e in wal.events(repo, &w).map_err(|e| e.to_string())? {
                if let Op::Review {
                    event,
                    verdict: Verdict::Approve,
                } = &e.op
                {
                    approved.insert(event.clone());
                }
            }
        }
        let unreviewed: Vec<&Event> = pending
            .iter()
            .filter(|e| matches!(e.op, Op::Write{..}|Op::Rename{..}|Op::Delete{..}))
            .filter(|e| !approved.contains(&e.id))
            .collect();
        if !unreviewed.is_empty() {
            return Err(format!(
                "review gate: {} unreviewed op(s)",
                unreviewed.len()
            ));
        }
    }
    // Base = current branch head, not any ws tip — merged ops apply
    // to where the branch is now.
    let base = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", &format!("refs/heads/{branch}")])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".into());
    let commit = sturn_git::snap_commit(repo, &base, &pending, message, branch)
        .map_err(|e| e.to_string())?;
    for ws in wss {
        wsi.create(repo, ws, &commit).map_err(|e| e.to_string())?;
        let _ = wal.append(
            repo,
            ws,
            &Event::new(
                ws,
                actor,
                Op::Snapshot {
                    message: message.into(),
                },
            ),
        );
    }
    Ok(commit)
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pieces the e2e can't reach: the daemon's
    //! per-iteration fan-out, the bridge's per-iteration webhook
    //! POST, socket subscribe/fanout helpers, and post_push.

    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr)
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sturn-cli-ut-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn repo(name: &str) -> PathBuf {
        let d = scratch(name);
        git(&d, &["init", "-q", "-b", "main"]);
        // Repo-local identity — wal append runs commit-tree.
        git(&d, &["config", "user.email", "t@t"]);
        git(&d, &["config", "user.name", "t"]);
        git(
            &d,
            &[
                "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init",
                "--allow-empty",
            ],
        );
        d
    }

    fn ws(repo: &Path, name: &str) {
        sturn_ws_git::GitWs
            .create(repo, name, "HEAD")
            .unwrap();
    }

    #[test]
    fn daemon_tick_fans_deltas_to_topics() {
        let repo_dir = repo("tick");
        let other = repo("tick-other");
        let repo = repo_dir;
        ws(&repo, "a");
        let wal = sturn_wal_git::GitWal::default();
        let bus = sturn_bus_redb::RedbBus::open(scratch("bus").join("b.redb")).unwrap();
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));
        // A live subscriber on the ops topic receives payloads.
        let (tx, rx) = mpsc::channel();
        subs.lock()
            .unwrap()
            .insert("tick|ops".into(), vec![tx]);
        let mut heads = HashMap::new();
        // ghost-repo: wal errors degrade to empty; "/" has no
        // file_name → rid falls back to "repo".
        let dirs = vec![
            repo.clone(),
            other.clone(),
            scratch("ghost-repo"),
            PathBuf::from("/"),
        ];

        wal.append(
            &repo,
            "a",
            &Event::new("a", "t", Op::Claim { glob: "x".into(), ttl_secs: 1 }),
        )
        .unwrap();
        wal.append(
            &repo,
            "a",
            &Event::new("a", "t", Op::Release { glob: "x".into() }),
        )
        .unwrap();
        wal.append(
            &repo,
            "a",
            &Event::new(
                "a",
                "t",
                Op::Review { event: "e".into(), verdict: Verdict::Reject },
            ),
        )
        .unwrap();
        wal.append(
            &repo,
            "a",
            &Event::new("a", "t", Op::Presence { status: "joined".into() }),
        )
        .unwrap();
        wal.append(
            &repo,
            "a",
            &Event::new("a", "t", Op::Intent { text: "i".into() }),
        )
        .unwrap();

        daemon_tick(
            &wal,
            &dirs,
            &repo,
            "tick",
            &mut heads,
            &bus,
            &subs,
        );
        // ops firehose got all 5; kind topics got theirs.
        let mut cursor = u64::MAX;
        let (mut c1, mut c2, mut c3) = (u64::MAX, u64::MAX, u64::MAX);
        assert_eq!(bus.poll("tick", topic::OPS, &mut cursor).unwrap().len(), 5);
        assert_eq!(bus.poll("tick", topic::CLAIMS, &mut c1).unwrap().len(), 2);
        assert_eq!(bus.poll("tick", topic::REVIEW, &mut c2).unwrap().len(), 1);
        assert_eq!(bus.poll("tick", topic::PRESENCE, &mut c3).unwrap().len(), 1);
        // Live socket subscriber saw the ops payloads.
        assert_eq!(rx.try_iter().count(), 5);

        // No new events → next tick publishes nothing.
        daemon_tick(&wal, &dirs, &repo, "tick", &mut heads, &bus, &subs);
        let mut c4 = u64::MAX;
        assert_eq!(bus.poll("tick", topic::OPS, &mut c4).unwrap().len(), 5);

        // A new event lands → only the delta publishes.
        wal.append(&repo, "a", &Event::new("a", "t", Op::Intent { text: "j".into() }))
            .unwrap();
        daemon_tick(&wal, &dirs, &repo, "tick", &mut heads, &bus, &subs);
        let mut c5 = u64::MAX;
        assert_eq!(bus.poll("tick", topic::OPS, &mut c5).unwrap().len(), 6);
    }

    #[test]
    fn handle_sub_streams_and_drops() {
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));
        let (a, mut b) = UnixStream::pair().unwrap();
        let subs2 = subs.clone();
        let h = std::thread::spawn(move || handle_sub(a, subs2));
        writeln!(b, "repo|ops").unwrap();
        b.flush().unwrap();
        // Registered under the topic key.
        for _ in 0..100 {
            if subs.lock().unwrap().contains_key("repo|ops") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let payload: Arc<Vec<u8>> = Arc::new(b"{}".to_vec());
        fanout(&subs, "repo", topic::OPS, &payload);
        let mut line = String::new();
        BufReader::new(&mut b).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "{}");
        // Dead receiver gets pruned on the next fanout.
        drop(b);
        fanout(&subs, "repo", topic::OPS, &payload);
        let _ = h.join();

        // Empty topic line → early return.
        let (a, mut b) = UnixStream::pair().unwrap();
        let subs3 = subs.clone();
        let h = std::thread::spawn(move || handle_sub(a, subs3));
        writeln!(b).unwrap();
        b.flush().unwrap();
        drop(b);
        h.join().unwrap();

        // Closed peer → read_line fails → early return.
        let (a, b) = UnixStream::pair().unwrap();
        drop(b);
        handle_sub(a, subs);
    }

    #[test]
    fn fanout_prunes_dead_senders() {
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel();
        let (dead, dead_rx) = mpsc::channel();
        drop(dead_rx);
        subs.lock().unwrap().insert("r|t".into(), vec![tx, dead]);
        let payload: Arc<Vec<u8>> = Arc::new(b"p".to_vec());
        fanout(&subs, "r", "t", &payload);
        assert_eq!(rx.try_iter().count(), 1);
        assert_eq!(subs.lock().unwrap()["r|t"].len(), 1);
        // No subscriber for the key — nothing to prune.
        fanout(&subs, "r", "other", &payload);
        drop(rx);
    }

    #[test]
    fn subscribe_streams_until_close() {
        let dir = scratch("sub");
        let sock = dir.join("bus.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut key = String::new();
            BufReader::new(conn.try_clone().unwrap())
                .read_line(&mut key)
                .unwrap();
            assert_eq!(key.trim(), "rid|ops");
            conn.write_all(b"{}\n").unwrap();
        });
        subscribe(&sock, "rid", "ops").unwrap();
        h.join().unwrap();

        // No daemon → connect error.
        assert!(subscribe(&sock, "rid", "ops").is_err());
    }

    fn http_server(status: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut rdr = BufReader::new(s.try_clone().unwrap());
            // Read headers then the body.
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                rdr.read_line(&mut line).unwrap();
                if let Some(v) = line.strip_prefix("Content-Length: ") {
                    len = v.trim().parse().unwrap();
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; len];
            std::io::Read::read_exact(&mut rdr, &mut body).unwrap();
            s.write_all(status.as_bytes()).unwrap();
            String::from_utf8_lossy(&body).into_owned()
        });
        (format!("http://127.0.0.1:{port}/"), h)
    }

    #[test]
    fn post_push_signs_and_posts() {
        let (url, h) = http_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        post_push(&url, "secret", "rid", "main", "deadbeef", "actor").unwrap();
        let body = h.join().unwrap();
        assert!(body.contains("refs/heads/main"));
        assert!(body.contains("deadbeef"));
        assert!(body.contains("rid"));

        // Non-2xx → error naming the status line.
        let (url, h) = http_server("HTTP/1.1 500 Nope\r\nContent-Length: 0\r\n\r\n");
        let err = post_push(&url, "s", "r", "main", "x", "a").unwrap_err();
        assert!(err.contains("500"), "{err}");
        let _ = h.join().unwrap();

        // Connection refused → error.
        assert!(post_push("http://127.0.0.1:1/", "s", "r", "m", "x", "a").is_err());
    }

    #[test]
    fn bridge_tick_posts_for_snapshot_events() {
        let repo = repo("bridge");
        ws(&repo, "a");
        let wal = sturn_wal_git::GitWal::default();
        let wsi = sturn_ws_git::GitWs;
        let mut heads = HashMap::new();

        // A snapshot event on ws `a` → one webhook POST.
        wal.append(
            &repo,
            "a",
            &Event::new(
                "a",
                "actor",
                Op::Snapshot { message: "m".into() },
            ),
        )
        .unwrap();
        // A non-snapshot event → ignored by the hook filter.
        wal.append(
            &repo,
            "a",
            &Event::new("a", "actor", Op::Intent { text: "i".into() }),
        )
        .unwrap();

        let (url, h) = http_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        bridge_tick(&repo, &wal, &wsi, &mut heads, &url, "s", "rid", "main");
        let body = h.join().unwrap();
        assert!(body.contains("actor"));

        // Head unchanged → no repost.
        bridge_tick(&repo, &wal, &wsi, &mut heads, &url, "s", "rid", "main");

        // Dead endpoint → tick survives (post errors are dropped).
        bridge_tick(&repo, &wal, &wsi, &mut heads, "http://127.0.0.1:1/", "s", "rid", "main");
    }

    // Under coverage, infinite drivers are stubbed; running the arms
    // covers their dispatch lines.
    #[cfg(coverage)]
    #[test]
    fn coverage_stubs_run() {
        std::env::set_var("BUPHA_WEBHOOK_SECRET", "s");
        let cli = Cli {
            repo: None,
            config: None,
            cmd: Cmd::Daemon,
        };
        assert!(run(&cli).is_ok());
        let cli = Cli {
            repo: None,
            config: None,
            cmd: Cmd::Bridge {
                to: "http://127.0.0.1:1/".into(),
                secret_env: "BUPHA_WEBHOOK_SECRET".into(),
            },
        };
        assert!(run(&cli).is_ok());
    }
}
