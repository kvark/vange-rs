//! HTTP-based data loader for the web build.
//!
//! Fetches zip archives from a GitHub release and mounts them into a
//! shared [`Vfs`]. No manifest: asset URLs are built from a fixed base
//! and a known level id. A missing asset produces a `NotFound` error
//! and the caller decides how to fall back.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

use crate::vfs::{Vfs, VfsError};

/// GitHub release that holds the data zips. Bump the tag to invalidate
/// every client's browser cache at once.
pub const RELEASE_BASE_URL: &str =
    "https://github.com/kvark/vange-rs/releases/download/data-0";

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

/// Fetch one release asset by filename (e.g. `"common.zip"`),
/// reporting incremental download progress to `progress`.
pub async fn fetch_asset(
    name: &str,
    progress: ProgressFn<'_>,
) -> Result<Vec<u8>, FetchError> {
    let url = format!("{}/{}", RELEASE_BASE_URL, name);
    fetch_bytes_streaming(name, &url, progress).await
}

async fn fetch_bytes_streaming(
    label: &str,
    url: &str,
    progress: ProgressFn<'_>,
) -> Result<Vec<u8>, FetchError> {
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

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

    // Drain the response body via its ReadableStream, accumulating
    // chunks and reporting cumulative bytes. We can't use array_buffer()
    // here because it gives no per-chunk callbacks.
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

    // Final notification with the true total (handles missing
    // Content-Length: caller can flip the bar to "complete").
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

/// Build the asset filename for a given level id. Convention: the
/// release holds one zip per level named `<id>.zip`.
pub fn level_archive_name(level_id: &str) -> String {
    format!("{}.zip", level_id)
}
