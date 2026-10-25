# Stumbler

A low-poly arcade racer built with Rust and wgpu, part racing game and part
open world. Twenty-three circuits sit on one big map of hills, lakes, forests,
desert and mountains, joined by roads: the home Stumbler Ring in the middle and
twenty-two real Formula 1 circuits around it, laid out on an even grid. Drive to any of them and race seven AI
rivals, or leave the tarmac and follow dirt trails out to lighthouses,
windmills, villages and mountain-top masts.

## Playing

```bash
cargo run --release
```

| Key | Gamepad | Action |
| --- | --- | --- |
| Up / W | Right trigger | Accelerate |
| Down / S | Left trigger | Brake, then reverse |
| Left, Right / A, D | Left stick | Steer |
| Space | A | Handbrake (kick the tail out) |
| R | Y | Reset onto the nearest road or trail (or upright where you are); in a race, back on the circuit where you'd got to |
| Enter | Start | Race (at a start line), or race again after finishing |
| T | LB | Time trial (at a start line) |
| G | RB | Five-race tour, starting here (at a start line) |
| Tab | D-pad up/down | Rivals' difficulty: easy, normal, hard (at a start line) |
| V | D-pad left/right | Car setup: balanced, grip (more wing, slower on the straights) or speed (less wing, looser in quick corners) |
| Backspace | B | Abandon the race / back to free roam |
| M | Back | World map |
| C | X | Change paint (from those you've unlocked) |
| F | | Fullscreen |
| Mouse wheel | | Camera distance |
| Esc | | Quit |

Drive up to any circuit's start line and press Enter to race. You start at
the back of the grid; get on the throttle just as the countdown ends for a
flying start (too early and you just bog down); the start lights on the
gantry count you down, red one by one, then green. Laps only count if you pass
through every sector, so cutting across the infield won't help, and you can
walk away from a race at any time. Boost pads on the straights give a short
burst of speed; tuck in close behind another car and its slipstream pulls
you along.

The rivals drive proper racing lines (wide in, clipping the apex, wide out),
each with a side of the line they like on the straights (so the field
spreads across the road rather than queueing down one line) before they
come back to it for the corners. Off the grid they keep straight in their
own lane for the launch, and only merge onto their lines once they're up
to speed. They
dive for the inside of the next corner to pass (or go round the outside
when the inside's shut), cover the inside when someone's right behind them,
and now and then outbrake themselves and run wide, more often under
pressure. Once a car's nose is alongside they leave it a car's width rather
than turning in on it. They keep their distance from a car
ahead (more when its brake lights come on) rather than running into the
back of it; one knocked off the road slows to turn back toward it, and one
that ends up beached, sunk or on its side is put back on the track after a
few seconds. The slowest rival starts on pole and the quickest further
back, so they race each other through the field. Each race they react to the start at
slightly different moments and are on slightly different form, so it isn't
always the same car out front; ones that fall well behind find a little
extra pace and ones far ahead ease off a touch, so races stay close. The
circuit's everyday traffic keeps off it while you race or run a time trial.
Circuits have wide, flat run-off beyond the kerbs, so running wide costs
time rather than sending you down a bank; tyre walls round the outside of
the corners, braking boards
counting down to the tight ones, sponsor hoardings along the straights, and
floodlights that throw pools of light on the track after dark.

Press G (or RB) at a start line for a tour: five races, starting here and
taking in the nearest circuits one after another, against the same seven
rivals for points (10, 8, 6, 5, 4, 3, 2, 1). Between rounds you drive to the next
circuit yourself; the guide at the top of the screen points the way. Walk
out of a tour race and the tour's over.

Press T (or LB) at a start line instead for a time trial: you alone, lap
after lap, with your time at each sector against your best lap, chasing the
ghost of that lap. Your best ghost on each circuit is kept (in a `ghosts` folder beside
the save file) so you can chase it again another day.

Press V (or D-pad left/right) at a start line to change the car's setup.
Grip runs more wing: it tops out around 267 km/h but carries more speed
through quick corners, so it suits twisty circuits. Speed trims the wing:
around 310 km/h, but it slides sooner, so it pays on long straights.
Balanced sits between the two. The difference is under a second a lap,
but it's real. Your choice is saved, and the rivals always run balanced.

Away from the circuits:

- Driving near a circuit or landmark discovers it. Every landmark has a dirt
  trail out to it from the road network (or from another trail).
- Every three landmarks found unlocks a new paint job.
- Eight secret places are hidden around the map, with no trail to them: a
  ring of standing stones, a drowned temple, an abandoned race car, a fairy
  ring, a buried stone face, an ancient oak, a shipwreck and hot springs.
  Lines of stone cairns lead out from the roads toward them. Some come
  alive after dark.
- The gold orbs scattered off-road each add a little engine power for good.
- The roads between circuits have edge lines (the centre line turns solid on
  bends), telegraph poles strung with wires, and a sign at each end naming
  the circuit ahead and how far it is. Wooden fingerposts mark where trails
  turn off to landmarks.
- A railway runs right round the edge of the map, with a second line up
  through the middle between the circuits. A steam train with four coaches
  makes its slow way round (the full tour takes over an hour and a half):
  across stone viaducts over the valleys and lakes, through tunnels under
  the mountains, and over level crossings whose barriers come down, lamps
  flashing, as it nears. Its windows glow after dark, and it will shove
  aside anything left on the line. The world map shows where it is.
- Farms line some of the roads through the grasslands: a farmhouse, red barn,
  silo, hay bales and a tractor, with hedged fields of wheat, rapeseed,
  sunflowers (all turned to the road), young crops and fresh ploughing, and
  cows or sheep in the pasture.

Discoveries, paint, best laps and best finishes are saved to
`~/.local/share/stumbler/` (override with `STUMBLER_SAVE=<file>`).

## Tuning the feel

Some of the feel can be adjusted without rebuilding. Put a `tuning.txt` next
to the save file (`~/.local/share/stumbler/tuning.txt`, or point
`STUMBLER_TUNING` at one) with `name value` lines (`#` starts a comment);
anything left out keeps its default, and it's read at start.

| Setting | Default | What it does |
| --- | --- | --- |
| `steer_in` | 4.5 | How fast held steering keys wind on lock (full lock per second) |
| `steer_return` | 8.0 | How fast the steering comes back when they're let go |
| `camera_distance` | 32 | Camera distance behind the car (the mouse wheel still adjusts it) |
| `camera_drop` | 0.1 | How much lower and closer the camera gets at top speed |
| `fov_at_speed` | 20 | Extra field of view at top speed (degrees) |
| `fov_boost` | 9 | Extra field of view under boost (degrees) |
| `speed_lines` | 1.0 | Wind streak brightness (0 turns them off) |
| `rival_pace` | 1.0 | Multiplies the rivals' pace, on top of the difficulty |
| `drift_angle` | 0.55 | Drift angle (radians) held with the wheel turned fully into it |
| `drift_grip` | 0.55 | Rear grip in a drift (lower slides more easily) |
| `slipstream` | 0.4 | Share of the drag a car right on another's tail is spared |

## Look

Flat-shaded low-poly throughout: every face takes a single colour (WGSL
`@interpolate(flat)` plus a little per-vertex jitter on the terrain), lit by a
strong key light and cool sky fill, with distance fog that melts into the sky
at the horizon. 4x MSAA keeps the polygon edges clean. Day turns to night over
six minutes; headlights come on after dark.

The land changes as you drive: meadows of grass and wildflower drifts, dense
conifer forest with ferns and fallen logs, autumn woods, a snow-capped range in
the north-east, snowy tundra, a volcano, and a desert of cacti and palm-fringed
oases. Lakes have beaches, clear shallows, reeds and lily pads, foam at the
waterline and sun glints off the waves. Trees and plants sway in the wind
(and whip about when you hit them), and deer, rabbits and butterflies live in
the meadows and woods, bolting when you come too close.

Now and then a shower blows through: the sky greys over, rain streaks past,
the road takes on a wet sheen and stays wet a while after the rain stops,
cars throw up spray, and everything has less grip (the rivals drive to it
too), so braking points and corner speeds change mid-race.

Out on the track, sliding tyres lay black rubber on the tarmac and tear dark
ruts in the grass, fading over a couple of minutes; wheels off the road
throw up clods and grit the colour of the ground; knocks between cars spray
sparks and chips of paint; brake lights glow as cars brake for a corner. At speed the camera drops lower, wind streaks
rush past, the view widens under boost and in a slipstream, and kerbs and
rough ground shake it. The engine climbs through the gears, tyres squeal on
tarmac and rumble on gravel, the wind roars as you go faster, and you hear
other cars' engines as they close in and pass, the note rising and falling.
The HUD calls the final lap and warns if you're going the wrong way round,
and fireworks go up as you take the flag (a proper show if you've won).

## Physics

`src/physics.rs` is an arcade rigid-body model: four ray-cast spring/damper
wheels, tyre forces at each contact patch with a friction circle, rear-wheel
drive with traction control, and speed-sensitive steering whose full lock
sits at the tyres' peak grip (so holding a key through a bend doesn't scrub
off the speed; the keys also ease the steering in). Pitch, roll and drifts
fall out of those forces. Kick the tail out with the handbrake (or a hard
turn-in) and keep the power on, and the car holds a drift: steer into it to
open the angle, let go to straighten up, counter-steer to snap out. A held
drift pays out a burst of boost as you straighten. Arcade helpers keep it
forgiving: yaw and drift-angle assist, a roll stabiliser, strong mid-air levelling so jumps off crests land
on the wheels, a cap on how fast the car can pitch or roll, and a hop back
onto the wheels if it ends up on its side or roof. Drive into deep water
and the car sinks; after a moment it is set back down on the shore nearby.
Shallows can be waded. Trees are soft: you
plough into them, losing speed fast (and spinning on a hard hit) while the
tree whips about. Buildings, big boulders, grandstands and other cars are
solid, with impulses at the contact point so glancing blows spin you.

## Code map

| Module | What it does |
| --- | --- |
| `main.rs` | Window, input, game loop, HUD layout |
| `gpu.rs`, `shaders/` | wgpu renderer and WGSL shaders (`common.wgsl` is prepended to each) |
| `tracks.rs`, `layout.rs` | Circuit outlines; fitting them (hairpin legs spread a road's width apart), placing them on the grid, linking them with roads |
| `circuit_data.rs` | Real circuit outlines, generated by `tools/gen_circuits.py` |
| `world.rs` | Builds the static world: road network, landmarks and trails, gates, boost pads |
| `road.rs` | Path network (circuits, roads, the railway, trails): resampled paths, height profiles, level crossings, nearest-path queries, road and track mesh |
| `landmarks.rs`, `trails.rs` | Siting landmarks and secrets where the terrain suits them; trails and cairns out to them |
| `secrets.rs` | The secret places, built by hand |
| `railway.rs` | Routing the railway between the circuits, the train's tour, level crossings, viaducts and tunnel portals, and the train itself |
| `roadside.rs` | Telegraph poles, road signs and trail fingerposts (lettered in the HUD's pixel font) |
| `trackside.rs` | Tyre walls, braking boards and sponsor hoardings round the circuits |
| `skids.rs`, `speedlines.rs` | Skid marks and ruts; wind streaks at speed |
| `weather.rs` | Showers, wet roads (less grip), falling rain |
| `farms.rs` | Farms along the roads: siting, fields and hedgerows (read by terrain, trees and cover), buildings and livestock |
| `chunks.rs`, `props.rs` | Streams terrain, trees, road surface, water and scenery in chunks on worker threads |
| `trees.rs`, `cover.rs` | Trees and shrubs per biome; grass, flowers, ferns, reeds and stones near the camera |
| `wildlife.rs` | Deer, rabbits and butterflies around the player |
| `terrain.rs` | Height field (flattened under roads) and biome colours |
| `physics.rs`, `collisions.rs` | Car physics and contacts |
| `ai_cars.rs` | AI drivers: speed profiles, pure-pursuit steering, racecraft |
| `race.rs` | Grid, countdown, laps, positions, results |
| `camera.rs`, `particles.rs`, `hud.rs`, `screen.rs` | Chase camera, smoke/dust, pixel-font HUD, minimap and world map |
| `progress.rs` | Save file |
| `gamepad.rs` | Gamepad steering, pedals and buttons (gilrs) |
| `tuning.rs` | The optional feel settings file |
| `car.rs`, `trees.rs`, `birds.rs`, `spectators.rs`, ... | Meshes and ambient life |

## Testing

```bash
cargo test --release
```

Includes physics behaviour tests (top speed, braking, cornering, drift
recovery, landings), collision tests, world checks (roads flat across, no
impossibly tight corners, a trail to every landmark), a headless
end-to-end race on the real map with the AI driving the player's car, a
keyboard-style driver (on/off keys only) lapping circuits cleanly without
taking off over crests, and slipstreaming.

`cargo test --release handling_telemetry -- --ignored --nocapture` prints
the car's numbers (acceleration, braking, cornering, turn-in, a held drift);
`keyboard_driving_report` compares keyboard and analogue laps on four circuits.

`cargo test --release render_ -- --ignored` draws `layout.ppm` (the whole map)
and `sketches.ppm` (each circuit as built, over its real outline in blue) and
prints each circuit's length against the real one and its shape error.

Dev aids:

- `STUMBLER_DEMO=1` lets the AI drive your car
- `STUMBLER_SCREENSHOT=out.ppm` saves one frame and exits (after
  `STUMBLER_SCREENSHOT_AT` seconds, default 2), printing the average frame time
- `STUMBLER_DAY_TIME=0.9` starts at a given time of day (0..1, 0.5 = noon)
- `STUMBLER_START=MONZA` starts at another circuit's line;
  `STUMBLER_START="OLD MILL"` starts at the end of a landmark's trail
- `STUMBLER_START=TRAIN` starts beside the line as the train comes;
  `STUMBLER_START=CROSSING` at a level crossing just before it does
- `STUMBLER_WEATHER=rain` (or `dry`) holds the weather one way
- `STUMBLER_RACE=1` lines up for a race straight away (`STUMBLER_RACE=trial`
  for a time trial, `STUMBLER_RACE=tour` for a tour)
- `STUMBLER_SCREENSHOT_MAP=1` opens the world map for the screenshot
- `STUMBLER_AT=x,z,yaw` starts anywhere (yaw in degrees, 0 = facing -Z)
- `STUMBLER_NO_HUD=1` hides the HUD, for clean screenshots
- Vsync is off (the frame rate is uncapped); `STUMBLER_VSYNC=1` turns it on.
  Screenshot runs report the frame time and triangle count
- `STUMBLER_WILDLIFE_CLOSE=1` spawns animals near the player
- `STUMBLER_PROFILE=1` times the render pass on the GPU (timestamp queries) and
  the CPU's update and draw; screenshot runs print the averages

## Credits

Circuit outlines come from [bacinger/f1-circuits](https://github.com/bacinger/f1-circuits)
(MIT, © 2019-2025 Tomislav Bacinger). To regenerate them:
`python3 tools/gen_circuits.py f1-circuits.geojson > src/circuit_data.rs`.
