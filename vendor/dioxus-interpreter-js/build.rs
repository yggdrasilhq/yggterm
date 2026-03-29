use std::fs;
use std::path::Path;

fn strip_class_fields(js: &mut String, class_prefix: &str, constructor_prefix: &str) {
    if let Some(class_start) = js.find(class_prefix) {
        let fields_start = class_start + class_prefix.len();
        if let Some(ctor_rel) = js[fields_start..].find(constructor_prefix) {
            let ctor_start = fields_start + ctor_rel;
            js.replace_range(fields_start..ctor_start, "");
        }
    }
}

fn postprocess_js(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let mut js = fs::read_to_string(path).expect("failed to read generated js");

    strip_class_fields(
        &mut js,
        "class BaseInterpreter{",
        "constructor(){",
    );
    strip_class_fields(
        &mut js,
        "class NativeInterpreter extends JSChannel_{",
        "constructor(baseUri,headless){",
    );
    js = js.replace(
        "class NativeInterpreter extends JSChannel_{constructor(baseUri,headless){super();",
        "class NativeInterpreter extends JSChannel_{constructor(baseUri,headless){super();this.queuedBytes=[];",
    );

    fs::write(path, js).expect("failed to write generated js");
}

fn main() {
    // If any TS files change, re-run the build script
    lazy_js_bundle::LazyTypeScriptBindings::new()
        .with_watching("./src/ts")
        .with_binding("./src/ts/set_attribute.ts", "./src/js/set_attribute.js")
        .with_binding("./src/ts/native.ts", "./src/js/native.js")
        .with_binding("./src/ts/core.ts", "./src/js/core.js")
        .with_binding("./src/ts/hydrate.ts", "./src/js/hydrate.js")
        .with_binding("./src/ts/patch_console.ts", "./src/js/patch_console.js")
        .with_binding(
            "./src/ts/initialize_streaming.ts",
            "./src/js/initialize_streaming.js",
        )
        .run();

    postprocess_js("./src/js/core.js");
    postprocess_js("./src/js/native.js");
}
