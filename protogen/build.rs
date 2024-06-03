fn main() {
    println!(
        "cargo:rerun-if-changed={}",
        concat![env!("CARGO_MANIFEST_DIR"), "/proto/system-event.proto"],
    );

    protobuf_codegen::Codegen::new()
        .pure()
        .cargo_out_dir("proto")
        .include("proto")
        .inputs(["proto/system-event.proto"])
        .run_from_script();
}
