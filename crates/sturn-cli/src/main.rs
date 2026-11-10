//! sturn — compat CLI driver. The engine is the `sturn` lib
//! (src/lib.rs); this binary is just `sturn::run()`.

fn main() {
    std::process::exit(sturn::run());
}
