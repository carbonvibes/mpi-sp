use std::path::PathBuf;

use fs_mutator::delta::FsDelta;
use libafl::{
    generators::NautilusContext,
    inputs::{Input, NautilusBytesConverter, NautilusInput, ToTargetBytes},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
struct CombinedInput {
    config: NautilusInput,
    rootfs: FsDelta,
}

impl libafl::inputs::Input for CombinedInput {
    fn generate_name(&self, _idx: Option<libafl::corpus::CorpusId>) -> String {
        "crash".into()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let crash_path = args.next().unwrap_or_else(|| {
        eprintln!("Usage: decode_crash <crash_file> [grammar.py]");
        std::process::exit(1);
    });
    let grammar_path = args.next().unwrap_or_else(|| {
        "/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py"
            .to_string()
    });

    let context: &'static NautilusContext =
        Box::leak(Box::new(NautilusContext::from_file(100, &grammar_path).unwrap_or_else(|e| {
            eprintln!("Failed to load grammar from {grammar_path}: {e}");
            std::process::exit(1);
        })));

    let input = CombinedInput::from_file(&crash_path).unwrap_or_else(|e| {
        eprintln!("Failed to decode {crash_path}: {e}");
        std::process::exit(1);
    });

    let mut conv = NautilusBytesConverter::new(context);
    let config_bytes = conv.to_target_bytes(&input.config);
    let config_json: serde_json::Value = serde_json::from_slice(&*config_bytes)
        .unwrap_or(serde_json::Value::String(
            String::from_utf8_lossy(&*config_bytes).to_string(),
        ));

    let out = serde_json::json!({
        "source": crash_path,
        "config": config_json,
        "rootfs_ops": input.rootfs,
    });

    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
