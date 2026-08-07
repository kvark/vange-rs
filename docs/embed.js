// Shared loader for the per-renderer routes (/, /voxel, /ray).
//
// Each route is a thin page that calls `boot()`; which terrain renderer
// it gets is decided in Rust from the URL path, see `terrain_choice` in
// bin/web/main.rs. Keeping the plumbing here means the routes differ
// only in their label.

/// Resolve a site-root path against *this module*, not the document.
///
/// The routes live in subdirectories, so a document-relative URL would
/// look for the data under `/voxel/data-0/` and quietly 404 into the
/// procedural fallback level — which still renders terrain, so it looks
/// like it worked. `embed.js` always sits at the site root, so
/// resolving against it is correct at any route depth.
const fromRoot = (path) => new URL(path, import.meta.url).href;

export function boot({ status } = {}) {
    const loading = document.getElementById('loading');
    const phase = document.getElementById('phase');
    const fill = document.getElementById('fill');
    const note = document.getElementById('note');

    // The Rust side calls these through `wasm_bindgen(catch)`, so any it
    // cannot find degrades to a no-op. They give the level download a
    // progress indicator.
    window.vangePhase = (label) => {
        phase.textContent = label;
        fill.classList.add('indet');
        fill.style.width = '';
    };
    window.vangeProgress = (label, loaded, total) => {
        phase.textContent = label;
        if (total > 0) {
            fill.classList.remove('indet');
            fill.style.width = (100 * loaded / total).toFixed(1) + '%';
            note.textContent = (loaded / 1048576).toFixed(1) + ' / '
                             + (total / 1048576).toFixed(1) + ' MB';
        }
    };
    window.vangeProgressDone = () => {
        // A recoverable miss earlier (a substituted level, say) may have
        // left the error styling on; we got here, so clear it.
        loading.classList.remove('error');
        loading.classList.add('gone');
        document.getElementById('canvas').focus();
    };
    window.vangeProgressError = (message) => {
        loading.classList.add('error');
        phase.textContent = message;
        fill.style.width = '0%';
        fill.classList.remove('indet');
    };

    window.vangeDataBase = fromRoot('data-0');
    // The level picker lives on the full version; here the level comes
    // from `#level=<id>` if present, otherwise the Rust default.
    window.vangeSelectedLevel = () => '';

    if (status) {
        phase.textContent = status;
    }

    // winit listens for keyboard events on the canvas itself, so it has
    // to hold focus for driving to work.
    const canvas = document.getElementById('canvas');
    canvas.addEventListener('mouseenter', () => canvas.focus());
    window.addEventListener('click', () => canvas.focus());

    return import(fromRoot('web.js')).then(({ default: init }) => init()).catch((e) => {
        // winit throws a control-flow exception to escape the event
        // loop; anything else is a real failure worth showing.
        if (!String(e).includes('Using exceptions for control flow')) {
            window.vangeProgressError(String(e));
            throw e;
        }
    });
}
