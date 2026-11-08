//! The store's build tool: compile a component for this engine.
//!
//!     harnix-wasm-compile <component.wasm> <out.cwasm>   # prints the fingerprint
//!     harnix-wasm-compile --fingerprint
//!
//! The artifact loads on any Harnix whose engine reports the printed
//! fingerprint, which is what the store files it under
//! (`wasm/<name>.<fingerprint>.cwasm`).

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let engine = harnix_wasm::WasmEngine::new()?;
    match args.as_slice() {
        [flag] if flag == "--fingerprint" => {}
        [input, output] => {
            let wasm = std::fs::read(input).with_context(|| format!("reading {input}"))?;
            let artifact = engine.compile(&wasm)?;
            std::fs::write(output, artifact).with_context(|| format!("writing {output}"))?;
        }
        _ => bail!("usage: harnix-wasm-compile <component.wasm> <out.cwasm> | --fingerprint"),
    }
    println!("{}", engine.fingerprint());
    Ok(())
}
