//! HTTP-based data loader for the web build.
//!
//! Fetches zip archives from the GitHub Pages site that serves the
//! demo and mounts them into a shared [`Vfs`]. The deploy workflow
//! (`.github/workflows/deploy-web.yml`) downloads release assets from
//! the `data-0` release into `docs/data-0/` at build time, so the
//! zips ship alongside the Pages HTML and load same-origin — no CORS
//! proxy, no GitHub API round-trip, no rate limits.
//!
//! The default base is `./data-0` (relative to the demo page). Set
//! `VANGERS_DATA_BASE` at build time to override, e.g. to point at a
//! CDN or a separate host.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

use crate::vfs::{Vfs, VfsError};

/// Base URL for data zips. Resolved relative to the demo page at
/// runtime, so the default works for GitHub Pages at any subpath.
pub fn data_base() -> &'static str {
    option_env!("VANGERS_DATA_BASE").unwrap_or("./data-0")
}

/// Archive that holds cross-level assets: resource models, sounds,
/// `game.lst`, `wrlds.dat`, palettes shared across worlds, etc.
pub const COMMON_ARCHIVE: &str = "common.zip";

/// Progress reports from a streaming fetch. `total` is `None` if the
/// server didn't send a `Content-Length` header.
pub type ProgressFn<'a> = &'a mut dyn FnMut(&str, u64, Option<u64>);

#[derive(Debug)]
pub enum FetchError {
    Network(String),
    NotFound(String),
    Http { status: u16, url: String },
    Vfs(VfsError),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            FetchError::Network(ref msg) => write!(f, "network error: {}", msg),
            FetchError::NotFound(ref url) => write!(f, "asset not found: {}", url),
            FetchError::Http { status, ref url } => {
                write!(f, "HTTP {} for {}", status, url)
            }
            FetchError::Vfs(ref e) => write!(f, "vfs: {}", e),
        }
    }
}
impl std::error::Error for FetchError {}

impl From<VfsError> for FetchError {
    fn from(e: VfsError) -> Self {
        FetchError::Vfs(e)
    }
}

fn js_err(v: wasm_bindgen::JsValue) -> FetchError {
    FetchError::Network(format!("{:?}", v))
}

/// Build the filename of a level's archive: `<id>.zip`.
pub fn level_archive_name(level_id: &str) -> String {
    format!("{}.zip", level_id)
}

/// Fetch one data asset by filename (e.g. `"common.zip"`), reporting
/// incremental download progress.
pub async fn fetch_asset(name: &str, progress: ProgressFn<'_>) -> Result<Vec<u8>, FetchError> {
    let base = data_base().trim_end_matches('/');
    let url = format!("{}/{}", base, name);
    fetch_bytes_streaming(name, &url, progress).await
}

async fn fetch_bytes_streaming(
    label: &str,
    url: &str,
    progress: ProgressFn<'_>,
) -> Result<Vec<u8>, FetchError> {
    let opts = RequestInit::new();
    opts.set_method("GET");
    // `same-origin` is the default, but we set it explicitly so that
    // the browser rejects the fetch quickly (rather than silently
    // opaque) if `data_base()` is mis-configured to a cross-origin URL.
    opts.set_mode(RequestMode::SameOrigin);

    let request = Request::new_with_str_and_init(url, &opts).map_err(js_err)?;
    let window = web_sys::window().ok_or_else(|| FetchError::Network("no window".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_err)?;
    let resp: Response = resp_value.dyn_into().map_err(js_err)?;

    let status = resp.status();
    if status == 404 {
        return Err(FetchError::NotFound(url.to_string()));
    }
    if !resp.ok() {
        return Err(FetchError::Http {
            status,
            url: url.to_string(),
        });
    }

    let total = resp
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok());

    // Initial 0% notification so the bar shows up before bytes arrive.
    progress(label, 0, total);

    // Drain the response body via its ReadableStream so we can report
    // incremental progress. `array_buffer()` would be simpler but gives
    // no per-chunk callbacks.
    let body = resp
        .body()
        .ok_or_else(|| FetchError::Network("response had no body".into()))?;
    let reader: ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|o| FetchError::Network(format!("not a default reader: {:?}", o)))?;

    let mut acc: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);

    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(js_err)?;
        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .map_err(js_err)?
            .as_bool()
            .unwrap_or(false);
        if done {
            break;
        }
        let value = js_sys::Reflect::get(&chunk, &"value".into()).map_err(js_err)?;
        let array: js_sys::Uint8Array = value.dyn_into().map_err(js_err)?;
        let n = array.byte_length() as usize;
        let start = acc.len();
        acc.resize(start + n, 0);
        array.copy_to(&mut acc[start..start + n]);
        progress(label, acc.len() as u64, total);
    }

    // Final notification with the true total.
    progress(label, acc.len() as u64, Some(acc.len() as u64));
    Ok(acc)
}

/// Fetch an asset and mount it into `vfs`, reporting download progress.
pub async fn fetch_and_mount(
    vfs: &mut Vfs,
    name: &str,
    progress: ProgressFn<'_>,
) -> Result<(), FetchError> {
    let bytes = fetch_asset(name, progress).await?;
    log::info!("Mounting {} ({} bytes) into VFS", name, bytes.len());
    vfs.mount_zip(&bytes)?;
    Ok(())
}
