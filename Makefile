WASM_TARGET = wasm32-unknown-unknown
WASM_BINDGEN_VERSION = 0.2.117
OUT_DIR = docs
PORT = 8080

.PHONY: web web-serve setup-web clean-web

setup-web:
	rustup target add $(WASM_TARGET)
	cargo install wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION)

## Build WASM and generate JS bindings
web:
	cargo build --target $(WASM_TARGET) --features web --bin web --release
	wasm-bindgen target/$(WASM_TARGET)/release/web.wasm \
		--out-dir $(OUT_DIR) --target web --no-typescript

## Serve the web build locally
web-serve: web
	@echo "Serving at http://localhost:$(PORT)"
	python3 -m http.server -d $(OUT_DIR) $(PORT)

## Remove generated JS/WASM from docs/
clean-web:
	rm -f $(OUT_DIR)/web.js $(OUT_DIR)/web_bg.wasm
