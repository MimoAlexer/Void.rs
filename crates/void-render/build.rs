use std::{env, fs, path::PathBuf};

fn main() {
    for (file, stage) in [
        ("background.vert", naga::ShaderStage::Vertex),
        ("background.frag", naga::ShaderStage::Fragment),
        ("terrain.vert", naga::ShaderStage::Vertex),
        ("terrain.frag", naga::ShaderStage::Fragment),
    ] {
        let path = format!("shaders/{file}");
        println!("cargo:rerun-if-changed={path}");
        let source = fs::read_to_string(&path).expect("shader source");
        let module = naga::front::glsl::Frontend::default()
            .parse(&naga::front::glsl::Options::from(stage), &source)
            .unwrap_or_else(|err| panic!("{path}: {err:?}"));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{path}: {err:?}"));
        let words = naga::back::spv::write_vec(
            &module,
            &info,
            &naga::back::spv::Options::default(),
            Some(&naga::back::spv::PipelineOptions {
                shader_stage: stage,
                entry_point: "main".into(),
            }),
        )
        .expect("SPIR-V generation");
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        fs::write(
            PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(format!("{file}.spv")),
            bytes,
        )
        .unwrap();
    }
}
