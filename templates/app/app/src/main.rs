//! The application: Harnix's own extensions, then yours, in order.

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("myapp", env!("CARGO_PKG_VERSION"))
        .extensions(harnix::first_party::standard())
        // .extension(my_space::Extension::default()).grant([..])
        .run()
}
