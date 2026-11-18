# shiny-tower
rust re-creation of simtower

The Bevy renderer displays the tower in a scrollable 2D view. Use **WASD** to move
the camera, **Q** to zoom out, and **E** to zoom in. On Mac, use two-finger
trackpad scrolling to pan the view. The camera stays within the map edges and
starts horizontally centered, focused on the bottom ten underground floors.
Press **Space** to pause or resume the last
selected simulation speed. The controls in the translucent top bar also let you
pause or set the speed to **1x**, **3x**, or **5x**. The bar shows the current
tick, time, day, quarter, year, and cash balance.

Choose a room from the **Build Room** menu, then click a floor to place it. The
selection stays active so you can keep building the same room type. A snapped,
translucent room preview follows the cursor; rejected builds show an error
tooltip beside the click. The separate **Build Steps** category places steps
across two floors for $5,000. Their full footprint must already have rooms or
floor segments on both floors; steps may overlap rooms but not elevator shafts or other sets of steps.
Elevator shafts also require an existing room or floor segment at their column on
every spanned floor, and neither structure creates floor segments automatically.

## Headless checks

The tower, simulation, worker, and runtime orchestration can be tested without
building Bevy or the renderer:

```sh
cargo test --lib --no-default-features --features headless
```

This compiles the headless library and runs tests for the tower, economy,
simulation, worker, and their supporting modules. The normal application keeps
the renderer enabled by default.
