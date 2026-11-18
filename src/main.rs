mod renderer;

/// Entry point that starts the simulation runtime.
fn main() {
    shiny_tower::runtime::run(renderer::run);
}
