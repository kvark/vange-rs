//! HTTP-based data loader for the web build.
//!
//! Fetches zip archives from a GitHub release and mounts them into a
//! shared [`Vfs`]. Asset URLs are discovered through the GitHub REST
//! API because the plain `releases/download/...` URLs return a 302
//! without CORS headers, which the browser rejects. The API endpoint
//! (`api.github.com`) sets `Access-Control-Allow-Origin: *` on both
//! the initial response and the redirect it issues to the CDN, so a
//! browser `fetch()` can follow the chain end-to-end.

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, ReadableStreamDefaultReader, Request, RequestInit, RequestMode, Response};

use crate::vfs::{Vfs, VfsError};

/// GitHub release tag whose assets hold the data zips. Bump to invalidate
/// every client's browser cache at once.
pub const RELEASE_TAG: &str = "data-0";

/// GitHub repository that owns the release.
pub const REPO: &str = "kvark/vange-rs";

/// Archive that holds cross-level assets: resource models, sounds,
/// `game.lst`, `wrlds.dat`, palettes shared across worlds, etc.
pub const COMMON_ARCHIVE: &str = "common.zip";

/// Progress reports from a streaming fetch. `total` is `None` if the
/// server didn't send a `Content-Length` header.
pub type ProgressFn<'a> = &'a mut dyn FnMut(&str, u64, Option<u64>);

thread_local! {
    /// Cache for the release's asset list, populated lazily on first
    /// use. One API round-trip per page load, shared by every asset
    /// fetch that follows.
    static ASSETS: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
}

#[derive(Debug)]
pub enum FetchError {
    Network(String),
    NotFound(String),
    Http { status: u16, url: String },
    Vfs(VfsError),
    BadJson(String),
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
            FetchError::BadJson(ref msg) => write!(f, "bad release metadata: {}", msg),
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

/// Fetch and parse the release metadata, returning a `name -> api_url`
/// map. Cached in a thread-local so repeated calls within one page
/// load don't re-hit the API (and don't consume rate-limit budget).
async fn fetch_asset_index() -> Result<HashMap<String, String>, FetchError> {
    if let Some(cached) = ASSETS.with(|c| c.borrow().clone()) {
        return Ok(cached);
    }

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        REPO, RELEASE_TAG
    );
    log::info!("Fetching release metadata: {}", url);

    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);
    let headers = Headers::new().map_err(js_err)?;
    // GitHub API requires an Accept header to return v3 JSON; without
    // it older clients get a non-JSON body on some error paths.
    headers.set("Accept", "application/vnd.github+json").map_err(js_err)?;
    opts.set_headers(&headers);

    let request = Request::new_with_str_and_init(&url, &opts).map_err(js_err)?;
    let window = web_sys::window().ok_or_else(|| FetchError::Network("no window".into()))?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_err)?;
    let resp: Response = resp_value.dyn_into().map_err(js_err)?;

    if resp.status() == 404 {
        return Err(FetchError::NotFound(url));
    }
    if !resp.ok() {
        return Err(FetchError::Http {
            status: resp.status(),
            url,
        });
    }

    let text_value = JsFuture::from(resp.text().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    let text = text_value
        .as_string()
        .ok_or_else(|| FetchError::BadJson("response body is not text".into()))?;

    let json = js_sys::JSON::parse(&text)
        .map_err(|_| FetchError::BadJson("not valid JSON".into()))?;
    let assets_val = js_sys::Reflect::get(&json, &"assets".into())
        .map_err(|_| FetchError::BadJson("missing `assets`".into()))?;
    let assets: js_sys::Array = assets_val
        .dyn_into()
        .map_err(|_| FetchError::BadJson("`assets` is not an array".into()))?;

    let mut map = HashMap::with_capacity(assets.length() as usize);
    for i in 0..assets.length() {
        let entry = assets.get(i);
        let name = js_sys::Reflect::get(&entry, &"name".into())
            .ok()
            .and_then(|v| v.as_string())
            .ok_or_else(|| FetchError::BadJson(format!("asset[{}].name missing", i)))?;
        let api_url = js_sys::Reflect::get(&entry, &"url".into())
            .ok()
            .and_then(|v| v.as_string())
            .ok_or_else(|| FetchError::BadJson(format!("asset[{}].url missing", i)))?;
        map.insert(name, api_url);
    }

    log::info!(
        "Release has {} asset(s): {}",
        map.len(),
        map.keys().cloned().collect::<Vec<_>>().join(", ")
    );

    ASSETS.with(|c| *c.borrow_mut() = Some(map.clone()));
    Ok(map)
}

/// Fetch one release asset by filename (e.g. `"common.zip"`),
/// reporting incremental download progress to `progress`.
pub async fn fetch_asset(name: &str, progress: ProgressFn<'_>) -> Result<Vec<u8>, FetchError> {
    let index = fetch_asset_index().await?;
    let api_url = index.get(name).cloned().ok_or_else(|| {
        FetchError::NotFound(format!(
            "asset `{}` not in release `{}`",
            name, RELEASE_TAG
        ))
    })?;
    fetch_bytes_streaming(name, &api_url, progress).await
}

async fn fetch_bytes_streaming(
    label: &str,
    url: &str,
    progress: ProgressFn<'_>,
) -> Result<Vec<u8>, FetchError> {
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);
    // `Accept: application/octet-stream` makes the GitHub API return
    // a 302 to the asset's CDN location. Without it, the API would
    // return the asset's JSON metadata instead.
    let headers = Headers::new().map_err(js_err)?;
    headers.set("Accept", "application/octet-stream").map_err(js_err)?;
    opts.set_headers(&headers);

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

