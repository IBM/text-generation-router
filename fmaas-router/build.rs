use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir("src/pb").unwrap_or(());
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir("src/pb")
        .include_file("mod.rs")
        .compile(
            &["../proto/generation.proto", "../proto/caikit_runtime_Nlp.proto", "../proto/caikit_runtime_info.proto"],
            &["../proto"],
        )
        .unwrap_or_else(|e| panic!("protobuf compilation failed: {}", e));

    Ok(())
}
