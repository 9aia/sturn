# Getting started

```sh
cargo install --git https://github.com/9aia/sturn sturn
cd your-repo
sturn init
```

`init` writes a `sturn.toml` and confirms the ref namespaces. From
here, workspaces are the unit of parallel work:

```sh
sturn ws new agent-7      # refs/sturn/ws/agent-7 → HEAD. No checkout.
sturn ws ls
```

Append ops to the workspace's log:

```sh
echo 'fn helper() { 42 }' | sturn write src/lib.rs --ws agent-7 --intent "add helper"
sturn claim "src/**" --ws agent-7
sturn log --ws agent-7    # JSONL replay of the WAL
```

Land the work as an ordinary commit:

```sh
sturn snap agent-7 -m "add helper"   # ops → real commit on main
sturn push                           # heads + refs/sturn/* → origin
```

Run the coordination daemon so other actors see ops live:

```sh
sturn daemon             # WAL → repo.<id>.{ops,claims,review,presence}
sturn deploy service     # systemd user unit
```

Next: [your first swarm](/tutorials/first-swarm).
