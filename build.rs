// Build script for NAPI-RS bindings
// Only runs when the 'napi' feature is enabled

fn main() {
    #[cfg(feature = "napi")]
    napi_build::setup();
}
