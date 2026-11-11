//! Embedded surface dispatch — the driver serving its own engine's
//! ops through `pavo-driver-kit`'s surface layer, no second binary
//! needed. `cli`/`mcp` argv verbs (feature-gated) plug this
//! [`Surface`] impl into `surface::cli::run` /
//! `surface::mcp::serve_stdio`.
//!
//! `op` reuses the same `cli.*` path wire queries take
//! (`ops::run_verb` re-execs `run` in the request's repo — output
//! stays captured off the serve stream). `surface` dispatches
//! `daemon`/`sub`/`bridge`/`mount` in-process: those verbs are
//! unbounded by design, so argv goes straight to `sturn::dispatch`
//! with stdio streaming.

use pavo_driver_kit::surface::{OpOutcome, OpSpec, Surface};
use serde_json::{Map, Value};

use crate::ops;

/// sturn's [`Surface`] — engine dispatch, already linked.
pub struct Engine;

impl Surface for Engine {
    /// Same path as the `cli.<verb>` wire query: re-exec `run` in
    /// the resolved repo, captured output → `OpOutcome`.
    fn op(&mut self, op: &OpSpec, params: &Map<String, Value>) -> OpOutcome {
        OpOutcome::from_response(&ops::run_op(&op.name, params))
    }

    /// `sturn <name> <args>` in-process — unbounded surfaces stream
    /// until they stop themselves (stdin EOF, signals), which is
    /// exactly why they're surfaces and not ops.
    fn surface(&mut self, name: &str, args: &[String]) -> i32 {
        let argv = std::iter::once("sturn".to_string())
            .chain(std::iter::once(name.to_string()))
            .chain(args.iter().cloned());
        sturn::dispatch(&sturn::parse_from(argv))
    }
}
