fn main() {
    println!(
        "cargo:rerun-if-changed={}",
        concat![env!("CARGO_MANIFEST_DIR"), "/proto/"],
    );

    for proto in ["system-event.proto", "trace-bundle.proto"] {
        println!(
            "cargo:rerun-if-changed={}/proto/{proto}",
            env!("CARGO_MANIFEST_DIR")
        );
    }

    protobuf_codegen::Codegen::new()
        .pure()
        .cargo_out_dir("proto")
        .include("proto")
        .inputs(["proto/system-event.proto", "proto/trace-bundle.proto"])
        .run_from_script();
}
