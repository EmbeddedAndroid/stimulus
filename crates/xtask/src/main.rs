use std::{fs, path::Path, process::Command};

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("gen") {
        eprintln!("usage: cargo xtask gen [--check]");
        std::process::exit(2);
    }
    let mut command = Command::new("python3");
    command.arg("tools/gen_ops.py");
    command.args(&args[1..]);
    match command.status() {
        Ok(status) if status.success() => {
            let schema = match lp_project::project::schema_document() {
                Ok(schema) => schema,
                Err(error) => {
                    eprintln!("failed to generate LPJ schema: {error}");
                    std::process::exit(1);
                }
            };
            let path = Path::new("docs/schemas/lpj-v1.json");
            if args.iter().any(|arg| arg == "--check") {
                if fs::read_to_string(path).ok().as_deref() != Some(schema.as_str()) {
                    eprintln!("generated artifact is stale: {}", path.display());
                    std::process::exit(1);
                }
            } else if let Err(error) = fs::write(path, schema) {
                eprintln!("failed to write {}: {error}", path.display());
                std::process::exit(1);
            }
        }
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("failed to run operation generator: {error}");
            std::process::exit(1);
        }
    }
}
