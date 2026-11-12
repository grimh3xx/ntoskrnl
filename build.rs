use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    // Mexe em HKLM, serviços e AppX: o exe pede administrador já ao abrir (UAC)
    // NTOSKRNL_AS_INVOKER=1: build de desenvolvimento sem pedir administrador (só para testar leitura)
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() && std::env::var_os("NTOSKRNL_AS_INVOKER").is_none() {
        embed_manifest(new_manifest("grimh3xx.ntoskrnl").requested_execution_level(ExecutionLevel::RequireAdministrator))
            .expect("não consegui embutir o manifesto");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=NTOSKRNL_AS_INVOKER");
    println!("cargo:rerun-if-changed=catalog");
}
