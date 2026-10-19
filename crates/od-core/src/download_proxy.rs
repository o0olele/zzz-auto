use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DownloadProxy {
    #[default]
    System,
    Direct,
    Custom {
        url: String,
    },
}

impl<'de> Deserialize<'de> for DownloadProxy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            mode: String,
            #[serde(default, deserialize_with = "present_url")]
            url: Option<String>,
        }
        fn present_url<'de, D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<String>, D::Error> {
            String::deserialize(deserializer).map(Some)
        }
        let fields = Fields::deserialize(deserializer)?;
        let invalid = || serde::de::Error::custom("下载代理结构无效");
        match fields.mode.as_str() {
            "system" if fields.url.is_none() => Ok(Self::System),
            "direct" if fields.url.is_none() => Ok(Self::Direct),
            "custom" => fields
                .url
                .map(|url| Self::Custom { url })
                .ok_or_else(invalid),
            _ => Err(invalid()),
        }
    }
}

impl DownloadProxy {
    pub fn validate(&self) -> Result<(), &'static str> {
        let Self::Custom { url: value } = self else {
            return Ok(());
        };
        if value.len() > 2048 || value.trim() != value || value.chars().any(char::is_control) {
            return Err("代理地址无效，请填写 HTTP 或 HTTPS 主机和端口");
        }
        let url =
            url::Url::parse(value).map_err(|_| "代理地址无效，请填写 HTTP 或 HTTPS 主机和端口")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err("代理只支持不含用户名、密码、路径参数、查询或片段的 HTTP / HTTPS 地址");
        }
        Ok(())
    }

    pub fn from_cli(value: String) -> Result<Self, &'static str> {
        let proxy = match value.as_str() {
            "system" => Self::System,
            "direct" => Self::Direct,
            _ => Self::Custom { url: value },
        };
        proxy.validate()?;
        Ok(proxy)
    }
}

// 包含仅本次使用的下载地址，不提供 Debug 或普通任务快照。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedDownloadRequest {
    pub source: crate::runtime_resources::RuntimeDownloadSource,
    pub proxy: DownloadProxy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_release: Option<crate::runtime_feed::RuntimeRelease>,
}
