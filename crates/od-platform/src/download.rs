use anyhow::{Result, ensure};
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub fn validate_https_url(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 8192 && !value.chars().any(char::is_control),
        "下载地址无效"
    );
    let url = reqwest::Url::parse(value).map_err(|_| anyhow::anyhow!("下载地址无效"))?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "下载地址须为不含用户名、密码或片段的 HTTPS 地址"
    );
    Ok(())
}

/// 传输与内容校验分开，离线测试可以替换为受控字节流。
pub trait ResourceDownloader {
    fn open(&self, url: &str) -> Result<Box<dyn Read>>;

    fn open_cancellable<'a>(
        &self,
        url: &str,
        _cancel: &'a AtomicBool,
    ) -> Result<Box<dyn Read + 'a>> {
        self.open(url)
    }
}

pub struct HttpsDownloader;

impl ResourceDownloader for HttpsDownloader {
    fn open(&self, url: &str) -> Result<Box<dyn Read>> {
        open_https(url, None)
    }

    fn open_cancellable<'a>(
        &self,
        url: &str,
        cancel: &'a AtomicBool,
    ) -> Result<Box<dyn Read + 'a>> {
        open_https(url, Some(cancel))
    }
}

fn wait_network<T>(
    runtime: &tokio::runtime::Runtime,
    cancel: Option<&AtomicBool>,
    future: impl std::future::Future<Output = reqwest::Result<T>>,
) -> Result<T> {
    runtime.block_on(async {
        tokio::pin!(future);
        loop {
            ensure!(
                cancel.is_none_or(|flag| !flag.load(Ordering::Acquire)),
                "资源下载已取消"
            );
            tokio::select! {
                result = &mut future => return Ok(result.map_err(|error| error.without_url())?),
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    })
}

struct HttpsReader<'a> {
    runtime: tokio::runtime::Runtime,
    response: reqwest::Response,
    cancel: Option<&'a AtomicBool>,
    pending: Vec<u8>,
    offset: usize,
}

impl Read for HttpsReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self.offset == self.pending.len() {
            let chunk = wait_network(&self.runtime, self.cancel, self.response.chunk())
                .map_err(std::io::Error::other)?;
            let Some(chunk) = chunk else {
                return Ok(0);
            };
            self.pending = chunk.to_vec();
            self.offset = 0;
        }
        let length = output.len().min(self.pending.len() - self.offset);
        output[..length].copy_from_slice(&self.pending[self.offset..self.offset + length]);
        self.offset += length;
        Ok(length)
    }
}

fn open_https<'a>(url: &str, cancel: Option<&'a AtomicBool>) -> Result<Box<dyn Read + 'a>> {
    validate_https_url(url)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let response = {
        let _entered = runtime.enter();
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(concat!("zzz-auto/", env!("CARGO_PKG_VERSION")))
            .build()?;
        wait_network(&runtime, cancel, client.get(url).send())?
            .error_for_status()
            .map_err(|error| error.without_url())?
    };
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "下载未返回完整文件"
    );
    Ok(Box::new(HttpsReader {
        runtime,
        response,
        cancel,
        pending: Vec::new(),
        offset: 0,
    }))
}
