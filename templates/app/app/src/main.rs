//! The application: Harnix's built-ins, then your modules, in order.

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("myapp", env!("CARGO_PKG_VERSION"))
        .with_builtins()
        .extensions(harnix::first_party::standard())
        // .module(my_space::Module::default())
        .run()
}
