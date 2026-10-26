// Embeds `tauri.conf.json`, the capabilities and `dist/` into the binary, and
// checks that every `bundle.externalBin` named there exists for this target.
fn main() {
    tauri_build::build()
}
