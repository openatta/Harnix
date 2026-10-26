//! `harnix` — the empty application: the built-ins and nothing else.
//!
//! An application's own binary is this file with its modules added
//! (`templates/app`).

fn main() -> anyhow::Result<()> {
    harnix::App::builder().with_builtins().run()
}
