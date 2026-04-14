//! HTTP-based data loader for the web build.
//!
//! Fetches zip archives from a GitHub release and mounts them into a
//! shared [`Vfs`]. No manifest: asset URLs are built from a fixed base
//! and a known level id. A missing asset produces a `NotFound` error
//! and the caller decides how to fall back.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

use crate::vfs::{Vfs, VfsError};

/// GitHub release that holds the data zips. Bump the tag to invalidate
/// every client's browser cache at once.
pub const RELEASE_BASE_URL: &str =
    "https://github.com/kvark/vange-rs/releases/download/data-0";

/// Archive that holds cross-level assets: resource models, sounds,
/// `game.lst`, `wrlds.dat`, palettes shared across worlds, etc.
pub const COMMON_ARCHIVE: &str = "common.zip";

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

/// Fetch one release asset by filename (e.g. `"common.zip"`).
pub async fn fetch_asset(name: &str) -> Result<Vec<u8>, FetchError> {
    let url = format!("{}/{}", RELEASE_BASE_URL, name);
    fetch_bytes(&url).await
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
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

    let buf_value = JsFuture::from(resp.array_buffer().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    let array = js_sys::Uint8Array::new(&buf_value);
    Ok(array.to_vec())
}

/// Fetch an asset and mount it into `vfs`.
pub async fn fetch_and_mount(vfs: &mut Vfs, name: &str) -> Result<(), FetchError> {
    let bytes = fetch_asset(name).await?;
    log::info!("Mounting {} ({} bytes) into VFS", name, bytes.len());
    vfs.mount_zip(&bytes)?;
    Ok(())
}

/// Build the asset filename for a given level id. Convention: the
/// release holds one zip per level named `<id>.zip`.
pub fn level_archive_name(level_id: &str) -> String {
    format!("{}.zip", level_id)
}
