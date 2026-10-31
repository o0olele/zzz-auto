use od_tools::lost_void_scene::{build_bundle, build_embedded_bundle};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Command};

fn test_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/lost-void-bundle-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

#[test]
fn explicit_reference_rejects_changed_sources_before_writing_output() {
    let root = test_root();
    let reference = root.join("reference");
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/lost-void/screen-source-lock.json"
    ))
    .unwrap();
    let path = reference.join(sources[0]["path"].as_str().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"changed reference source").unwrap();
    let output = root.join("output");

    let error = build_bundle(&reference, &root.join("models"), &output).unwrap_err();

    assert!(
        error.to_string().contains("迷失之地画面来源已变化"),
        "{error:#}"
    );
    assert!(!output.exists());
}

#[test]
fn embedded_bundle_rejects_changed_ocr_before_writing_output() {
    let root = test_root();
    let models = root.join("models");
    let lock = od_tools::models::locked_models().unwrap();
    let ocr = lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .unwrap();
    let file = ocr
        .files
        .iter()
        .find(|file| file.bundle_path.is_some())
        .unwrap();
    let path = models.join("ppocrv5").join(&file.path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"changed OCR model").unwrap();
    let output = root.join("output");

    let error = build_embedded_bundle(&models, &output).unwrap_err();

    assert!(
        error.to_string().contains("迷失之地画面 OCR 文件不匹配"),
        "{error:#}"
    );
    assert!(!output.exists());
}

#[test]
#[ignore = "需要固定 OCR 缓存 sdk/models"]
fn cli_builds_from_embedded_assets_without_reference_checkout() {
    let root = test_root();
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/models")
        .canonicalize()
        .unwrap();
    let output = root.join("output");
    // 从没有参考仓的目录运行，覆盖桌面准备脚本不传 --reference 的调用。
    let result = Command::new(env!("CARGO_BIN_EXE_od-tools"))
        .current_dir(&root)
        .arg("lost-void-scene-bundle")
        .arg("--models")
        .arg(models)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(output.join("lost_void_scene-manifest.json")).unwrap(),
    )
    .unwrap();
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/lost-void/screen-source-lock.json"
    ))
    .unwrap();
    assert_eq!(manifest["sources"], sources);
    assert_eq!(manifest["accepted"], false);
    for file in manifest["files"].as_array().unwrap() {
        let bytes = std::fs::read(output.join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(file["bytes"], bytes.len() as u64);
        assert_eq!(file["sha256"], hex::encode(Sha256::digest(bytes)));
    }
}
