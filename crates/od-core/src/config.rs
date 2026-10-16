use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use ts_rs::TS;
use uuid::Uuid;

pub const CONFIG_SCHEMA: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GeneralSettings {
    pub capture_method: crate::CaptureMethod,
    pub mail: crate::mail::MailConfig,
}

impl GeneralSettings {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.mail.validate().map_err(invalid)
    }

    // 与协议一起生成表单定义；前端不维护另一份字段名、可选值与数值范围。
    pub fn fields() -> serde_json::Value {
        use crate::{CaptureMethod, mail::MailConfig};
        let options: Vec<_> = [
            CaptureMethod::FramePool,
            CaptureMethod::PrintWindow,
            CaptureMethod::Dxgi,
            CaptureMethod::ScreenDc,
            CaptureMethod::Gdi,
        ]
        .into_iter()
        .map(|value| serde_json::json!({ "value": value, "label": value.name() }))
        .collect();
        serde_json::json!([
            { "key": "capture_method", "kind": "select", "label": "默认截图方式", "options": options },
            { "key": "timeout_ms", "kind": "integer", "label": "默认邮件超时（ms）", "min": 1, "max": MailConfig::MAX_TIMEOUT_MS },
            { "key": "start_delay_ms", "kind": "integer", "label": "默认启动 / 继续延时（ms）", "min": 0, "max": MailConfig::MAX_START_DELAY_MS }
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GroupApp {
    pub app_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppGroup {
    pub id: String,
    #[serde(default)]
    pub run_policy: crate::daily::RunPolicy,
    pub apps: Vec<GroupApp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AccountInstance {
    pub id: u32,
    pub name: String,
    pub active_in_od: bool,
    #[serde(default)]
    pub game_region: crate::daily::GameRegion,
    pub groups: Vec<AppGroup>,
}

// 仅包含可公开的编排字段；账号凭据和未迁移 YAML 不进入协议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub schema: u32,
    pub revision: u32,
    // v1 文件缺少整个设置对象时兼容默认值；对象存在时仍严格校验字段。
    #[serde(default)]
    pub general: GeneralSettings,
    pub active_instance: u32,
    pub instances: Vec<AccountInstance>,
    pub import_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigChange {
    SetGameRegion {
        instance_id: u32,
        game_region: crate::daily::GameRegion,
    },
    SetRunPolicy {
        instance_id: u32,
        group_id: String,
        run_policy: crate::daily::RunPolicy,
    },
    SaveGeneral {
        settings: GeneralSettings,
    },
    CreateInstance {
        name: String,
    },
    UpdateInstance {
        instance_id: u32,
        name: String,
        active_in_od: bool,
    },
    SwitchInstance {
        instance_id: u32,
    },
    SaveGroup {
        instance_id: u32,
        group_id: String,
        apps: Vec<GroupApp>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("配置修订冲突，请读取最新配置后重试")]
    Conflict(Box<AccountConfig>),
    #[error("其他进程正在修改配置，请稍后重试")]
    Locked,
    #[error("该数据目录正被引擎或导入工具使用，请先退出相关程序")]
    InUse,
    #[error("{0}")]
    Invalid(String),
    #[error("配置文件读写失败：{0}")]
    Io(#[from] std::io::Error),
}

fn invalid(message: &str) -> ConfigError {
    ConfigError::Invalid(message.into())
}

pub fn default_group() -> AppGroup {
    let mut apps = crate::applications().expect("内置应用登记无效");
    apps.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));
    AppGroup {
        id: "one_dragon".into(),
        run_policy: crate::daily::RunPolicy::default(),
        apps: apps
            .into_iter()
            .filter(|app| app.default_group)
            .map(|app| GroupApp {
                app_id: app.id,
                enabled: false,
            })
            .collect(),
    }
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA,
            revision: 0,
            general: GeneralSettings::default(),
            active_instance: 1,
            instances: vec![AccountInstance {
                id: 1,
                name: "默认账号".into(),
                active_in_od: true,
                game_region: crate::daily::GameRegion::default(),
                groups: vec![default_group()],
            }],
            import_id: None,
        }
    }
}

impl AccountConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema != CONFIG_SCHEMA {
            return Err(invalid("账号配置版本不支持"));
        }
        self.general.validate()?;
        if self.instances.is_empty() || self.instances.len() > 128 {
            return Err(invalid("账号数量必须为 1..128"));
        }
        if let Some(id) = &self.import_id
            && (id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(invalid("导入指纹无效"));
        }
        let mut ids = HashSet::new();
        for instance in &self.instances {
            if instance.id == 0 || instance.id > 9999 || !ids.insert(instance.id) {
                return Err(invalid("账号编号重复或不在 1..9999 范围内"));
            }
            validate_name(&instance.name)?;
            if instance.groups.is_empty() || instance.groups.len() > 64 {
                return Err(invalid("每个账号的应用组数量必须为 1..64"));
            }
            let mut groups = HashSet::new();
            for group in &instance.groups {
                if !valid_identifier(&group.id) || !groups.insert(&group.id) {
                    return Err(invalid("应用组标识无效或重复"));
                }
                if group.apps.len() > 256 {
                    return Err(invalid("应用组最多包含 256 项"));
                }
                let mut apps = HashSet::new();
                for app in &group.apps {
                    if !valid_identifier(&app.app_id) || !apps.insert(&app.app_id) {
                        return Err(invalid("应用标识无效或重复"));
                    }
                }
            }
        }
        if !ids.contains(&self.active_instance) {
            return Err(invalid("当前账号不存在"));
        }
        Ok(())
    }

    fn change(&mut self, change: ConfigChange) -> Result<(), ConfigError> {
        match change {
            ConfigChange::SetGameRegion {
                instance_id,
                game_region,
            } => {
                self.instance_mut(instance_id)?.game_region = game_region;
            }
            ConfigChange::SetRunPolicy {
                instance_id,
                group_id,
                run_policy,
            } => {
                self.instance_mut(instance_id)?
                    .groups
                    .iter_mut()
                    .find(|group| group.id == group_id)
                    .ok_or_else(|| invalid("应用组不存在"))?
                    .run_policy = run_policy;
            }
            ConfigChange::SaveGeneral { settings } => {
                self.general = settings;
            }
            ConfigChange::CreateInstance { name } => {
                validate_name(&name)?;
                let id = (1..=9999)
                    .find(|id| !self.instances.iter().any(|i| i.id == *id))
                    .ok_or_else(|| invalid("没有可用的账号编号"))?;
                self.instances.push(AccountInstance {
                    id,
                    name,
                    active_in_od: true,
                    game_region: crate::daily::GameRegion::default(),
                    groups: vec![default_group()],
                });
            }
            ConfigChange::UpdateInstance {
                instance_id,
                name,
                active_in_od,
            } => {
                let instance = self.instance_mut(instance_id)?;
                instance.name = name;
                instance.active_in_od = active_in_od;
            }
            ConfigChange::SwitchInstance { instance_id } => {
                self.instance_mut(instance_id)?;
                self.active_instance = instance_id;
            }
            ConfigChange::SaveGroup {
                instance_id,
                group_id,
                apps,
            } => {
                let group = self
                    .instance_mut(instance_id)?
                    .groups
                    .iter_mut()
                    .find(|g| g.id == group_id)
                    .ok_or_else(|| invalid("应用组不存在"))?;
                // 首批编辑仅排序 / 启停已有项，避免删除尚未迁移的插件配置。
                let before: HashSet<_> = group.apps.iter().map(|a| &a.app_id).collect();
                let after: HashSet<_> = apps.iter().map(|a| &a.app_id).collect();
                if before != after || apps.len() != group.apps.len() {
                    return Err(invalid("保存应用组必须保留全部原有应用"));
                }
                group.apps = apps;
            }
        }
        self.validate()
    }

    fn instance_mut(&mut self, id: u32) -> Result<&mut AccountInstance, ConfigError> {
        self.instances
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| invalid("账号不存在"))
    }
}

fn validate_name(name: &str) -> Result<(), ConfigError> {
    if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(invalid("账号名称须为 1..80 个字符，不能包含控制字符"));
    }
    Ok(())
}

pub fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.')
        && id != "."
        && id != ".."
}

#[derive(Clone)]
pub struct ConfigStore {
    directory: PathBuf,
}

// 用户数据不能经目录联接或文件链接写回旧配置。
pub fn reject_links(path: &Path) -> Result<(), ConfigError> {
    let mut current = PathBuf::new();
    for part in path.components() {
        if matches!(part, std::path::Component::ParentDir) {
            return Err(invalid("数据路径不能包含 .."));
        }
        current.push(part);
        if matches!(part, std::path::Component::Prefix(_)) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        };
        let mut link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            link |= metadata.file_attributes() & 0x400 != 0;
        }
        if link {
            return Err(invalid("数据路径不能包含符号链接或目录联接"));
        }
    }
    Ok(())
}

impl ConfigStore {
    // 引擎整个生命周期与离线导入互斥；文件句柄关闭或进程退出后自动释放。
    pub fn acquire_owner(&self) -> Result<File, ConfigError> {
        reject_links(&self.directory.join("owner.lock"))?;
        fs::create_dir_all(&self.directory)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("owner.lock"))?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => ConfigError::InUse,
            std::fs::TryLockError::Error(error) => ConfigError::Io(error),
        })?;
        Ok(file)
    }

    pub fn new(data: &Path) -> Result<Self, ConfigError> {
        if !data.is_absolute() {
            return Err(invalid("数据目录必须为绝对路径"));
        }
        reject_links(&data.join("config"))?;
        Ok(Self {
            directory: data.join("config"),
        })
    }

    pub fn read(&self) -> Result<AccountConfig, ConfigError> {
        let path = self.directory.join("accounts.yml");
        reject_links(&path)?;
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AccountConfig::default());
            }
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > MAX_CONFIG_BYTES {
            return Err(invalid("账号配置过大"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid("账号配置过大"));
        }
        let config: AccountConfig = serde_yaml::from_slice(&bytes)
            .map_err(|_| invalid("账号配置损坏或包含不支持的字段，请从备份恢复"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn update(
        &self,
        revision: u32,
        change: ConfigChange,
    ) -> Result<AccountConfig, ConfigError> {
        let _lock = self.lock()?;
        let mut config = self.read()?;
        if config.revision != revision {
            return Err(ConfigError::Conflict(Box::new(config)));
        }
        config.change(change)?;
        self.save(&mut config)?;
        Ok(config)
    }

    pub fn install_import(&self, mut config: AccountConfig) -> Result<AccountConfig, ConfigError> {
        config.validate()?;
        if config.import_id.is_none() {
            return Err(invalid("导入必须包含源文件指纹"));
        }
        let _lock = self.lock()?;
        let current = self.read()?;
        if current.import_id == config.import_id {
            return Ok(current);
        }
        if current.revision != 0 || self.directory.join("accounts.yml").try_exists()? {
            return Err(invalid(
                "已有账号配置，首批导入不能覆盖；请使用新的数据目录",
            ));
        }
        config.revision = current.revision;
        self.save(&mut config)?;
        Ok(config)
    }

    fn lock(&self) -> Result<File, ConfigError> {
        reject_links(&self.directory.join("accounts.lock"))?;
        fs::create_dir_all(&self.directory)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.directory.join("accounts.lock"))?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => ConfigError::Locked,
            std::fs::TryLockError::Error(error) => ConfigError::Io(error),
        })?;
        Ok(lock)
    }

    fn save(&self, config: &mut AccountConfig) -> Result<(), ConfigError> {
        config.revision = config
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("配置修订号已用尽"))?;
        let path = self.directory.join("accounts.yml");
        reject_links(&path)?;
        let temporary = self
            .directory
            .join(format!("accounts-{}.tmp", Uuid::new_v4()));
        let bytes = serde_yaml::to_string(config).map_err(|_| invalid("账号配置无法序列化"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid("账号配置过大，未保存"));
        }
        let result = (|| -> Result<(), ConfigError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(bytes.as_bytes())?;
            file.sync_all()?;
            if path.try_exists()? {
                let backups = self.directory.join("backups");
                reject_links(&backups)?;
                fs::create_dir_all(&backups)?;
                let backup = backups.join(format!(
                    "accounts-r{}-{}.yml",
                    config.revision - 1,
                    Uuid::new_v4()
                ));
                fs::copy(&path, &backup)?;
                OpenOptions::new().write(true).open(&backup)?.sync_all()?;
            }
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}
