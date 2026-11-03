# TODO — sturn

- [ ] T-005 Port vocabulary docs: declare refs/sha the port contract; reword Wal::append (linearizable, not update-ref), Wal::head (opaque cursor), Vcs::push · status:in-progress · pri:low · owner:devin · created:2026-11-03
- [ ] T-006 De-concretize sturn-cli: handlers take \&dyn Wal/\&dyn Ws (mcp already does); Vcs::sync gains remote param; move main.rs:1535 raw rev-parse behind a port · status:claimed · pri:low · owner:devin · created:2026-11-03
