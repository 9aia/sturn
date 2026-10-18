//! The agent surface — the tool vocabulary every sturn agent sees.
//! No git, no filesystem: the VCS stops being a tool the agent
//! operates and becomes the medium it lives in.
//!
//! MCP wire-up lands in M3; this is the contract that shapes it.

/// One tool in the agent vocabulary.
pub struct Tool {
    /// MCP tool name.
    pub name: &'static str,
    /// What the agent reads before calling it.
    pub about: &'static str,
}

/// The whole AX — nine tools instead of git's thirty-something.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "ws_read",
        about: "Blob contents at ws:path — straight from the ODB",
    },
    Tool {
        name: "ws_write",
        about: "Append a write op — visible on the ops topic in ms",
    },
    Tool {
        name: "ws_glob",
        about: "Path listing for a ws tree",
    },
    Tool {
        name: "ws_grep",
        about: "Content search across a ws tree",
    },
    Tool {
        name: "claim",
        about: "Advisory lock on a path glob — conflict answer now, not at merge",
    },
    Tool {
        name: "subs",
        about: "Live topic feed — claims, ops, review, presence",
    },
    Tool {
        name: "snap",
        about: "Squash ws ops into a real commit on the branch",
    },
    Tool {
        name: "intent",
        about: "Record *why* — provenance as data, not a commit message",
    },
    Tool {
        name: "review",
        about: "Approve/reject an event — continuous review",
    },
];
