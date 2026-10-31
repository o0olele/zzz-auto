use od_core::runtime_resources::{ResourceSelection, RuntimeResourceRequest};
use std::path::{Path, PathBuf};
use zzz_engine::runtime_resources;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> anyhow::Result<Self> {
        let root = project()
            .join("artifacts")
            .join(format!("resource-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("config"))?;
        std::fs::create_dir_all(root.join("runtime-resources/versions"))?;
        for id in ["a".repeat(64), "b".repeat(64)] {
            let old = root.join("runtime-resources/versions").join(id);
            std::fs::create_dir_all(&old)?;
            std::fs::write(old.join("old-resource-marker"), b"keep old package")?;
        }
        std::fs::write(
            root.join("config/accounts.yml"),
            b"keep existing account bytes\n",
        )?;
        std::fs::write(
            root.join("runtime-resources/selection.json"),
            serde_json::to_vec(&ResourceSelection {
                schema_version: 1,
                current: Some("a".repeat(64)),
                previous: Some("b".repeat(64)),
                can_rollback: true,
            })?,
        )?;
        Ok(Self(root))
    }
    fn selection(&self) -> PathBuf {
        self.0.join("runtime-resources/selection.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let parent = project().join("artifacts").canonicalize().unwrap();
        assert!(root.starts_with(&parent) && root != parent);
        let _ = std::fs::remove_dir_all(root);
    }
}
fn project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
#[ignore = "需要完整生成的当前运行资源；不加载 SDK 或操作游戏"]
fn use_bundled_recovers_from_two_unusable_versions_and_preserves_data() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let bundled = project().join("resources");
    assert!(runtime_resources::resolve(&fixture.0, &bundled).is_err());
    assert!(runtime_resources::rollback(&fixture.0, &bundled).is_err());
    let before = std::fs::read(fixture.selection())?;
    let request = RuntimeResourceRequest::UseBundled {
        revision: runtime_resources::catalog(&fixture.0)?.revision,
    };
    assert!(request.changes_selection() && request.requires_engine_exit());
    let preview = runtime_resources::preview_change(&fixture.0, &bundled, &request)?;
    assert_eq!(
        std::fs::read(fixture.selection())?,
        before,
        "预览不能写选择"
    );
    assert!(preview.current.is_none());
    assert_eq!(preview.previous, Some("a".repeat(64)));
    let applied = runtime_resources::apply_change(&fixture.0, &bundled, &request)?;
    assert_eq!(applied, preview);
    assert_eq!(runtime_resources::resolve(&fixture.0, &bundled)?, bundled);
    assert_eq!(
        std::fs::read(fixture.0.join("config/accounts.yml"))?,
        b"keep existing account bytes\n"
    );
    for id in ["a".repeat(64), "b".repeat(64)] {
        assert_eq!(
            std::fs::read(
                fixture
                    .0
                    .join("runtime-resources/versions")
                    .join(id)
                    .join("old-resource-marker")
            )?,
            b"keep old package"
        );
    }
    // 旧版本仍须通过原有校验，不能因为恢复入口而绕过兼容性要求。
    assert!(runtime_resources::rollback(&fixture.0, &bundled).is_err());
    assert_eq!(
        runtime_resources::status(&fixture.0, &bundled)?.selection,
        applied
    );
    Ok(())
}

#[test]
fn use_bundled_rejects_invalid_target_stale_preview_and_busy_owner() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let bundled = fixture.0.join("empty-bundled-resources");
    std::fs::create_dir(&bundled)?;
    let before = std::fs::read(fixture.selection())?;
    let request = RuntimeResourceRequest::UseBundled {
        revision: runtime_resources::catalog(&fixture.0)?.revision,
    };
    assert!(runtime_resources::apply_change(&fixture.0, &bundled, &request).is_err());
    assert_eq!(std::fs::read(fixture.selection())?, before);
    let stale = RuntimeResourceRequest::UseBundled {
        revision: "0".repeat(64),
    };
    assert!(
        runtime_resources::preview_change(&fixture.0, &bundled, &stale)
            .unwrap_err()
            .to_string()
            .contains("选择已变化")
    );
    let store = od_core::config::ConfigStore::new(&fixture.0)?;
    let _owner = store.acquire_owner()?;
    assert!(runtime_resources::apply_change(&fixture.0, &bundled, &request).is_err());
    assert_eq!(std::fs::read(fixture.selection())?, before);
    Ok(())
}
