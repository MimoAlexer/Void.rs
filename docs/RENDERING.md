# Vulkan implementation and remaining qualification

The renderer uses ash directly, Vulkan 1.4, a graphics/presentation queue, and two
frames in flight. It selects a discrete GPU when available. Current terrain is a
diagnostic colored block mesh; it does not implement vanilla models, materials,
textures, lighting, transparency, or visual parity.

## Lifetime and synchronization contract

- Keep the native window alive until `Renderer` is dropped. An owning application
  must drop its renderer before its `Window`; field declaration order alone can
  otherwise destroy the window first.
- `prepare_frame` waits for the current slot's fence before mutable per-frame
  buffers are reused. `update_mesh`, `update_entities`, and `render` also ensure
  preparation when called independently. Static terrain and dynamic entities use
  separate buffers. Each new prepared frame starts with empty dynamic geometry.
- Acquisition semaphores/fences are indexed by frame slot; render-finished
  semaphores are indexed by swapchain image. This follows the
  [Khronos semaphore reuse guidance](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html).
- Depth reuse includes a late-fragment-tests write to early-fragment-tests
  read/write dependency. Acquisition waits cover early depth and color output.
- Screenshot copies explicitly make transfer writes visible to host reads before
  the submission fence is awaited. Host-coherent allocation avoids a separate
  mapped-memory invalidation, not this dependency. See the
  [Khronos synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html).
- Only supported RGBA8/BGRA8 formats with transfer-source surface usage can be
  captured. Unsupported capture requests return an error. BGRA output is reordered
  for PNG. Screenshot size calculations are checked before allocation.
- Swapchain recreation idles submitted device work, destroys tracked views/depth
  images/framebuffers, and recreates them. The render pass/pipelines remain
  compatible while the surface format is unchanged; a format change currently
  requires restarting the client.
- Allocated buffers, partially created shader pipelines, and screenshot buffers
  are cleaned up on ordinary Vulkan/I/O failure paths. Rendering errors should be
  treated as fatal for the current renderer rather than resuming a partly failed
  frame.

The shader build uses Naga 29's default SPIR-V Y adjustment, which flips only Y.
The client uses glam's `[0,1]` depth projection and a positive-height Vulkan
viewport. Adding another Y flip or OpenGL depth remap would be incorrect.

`Renderer::is_frame_ready` exposes a nonblocking fence-status check. Coordinated
frontends can keep processing input while a slot is busy, then request a redraw
when it becomes available. `FrameTiming::fence_wait_us` separately records CPU
wall time actually spent in `prepare_frame`'s fence wait. This differs from the
conventional baseline that starts the redraw and blocks on the fence. Use the
same frame cap and queue depth in both modes; no extra artificial delay belongs
in the baseline. This narrow distinction requires a real GPU-backpressure
comparison before any latency improvement is claimed.

## Open limitations

- Swapchain/presentation teardown uses `device_wait_idle`. Unextended Vulkan does
  not expose a presentation fence, so this is the common practical fallback rather
  than a formal presentation-completion guarantee. Add a supported
  `VK_EXT_swapchain_maintenance1` / `VK_KHR_swapchain_maintenance1` present-fence
  path before claiming fully qualified presentation lifetime handling.
- Acquisition still uses an infinite timeout and can block after CPU input was
  sampled. Frame-fence coordination alone cannot remove acquisition latency.
- Terrain buffers currently use host-visible coherent memory and whole-mesh
  uploads. GPU-local allocations, bounded section uploads, persistent staging,
  occlusion work, and pipeline-cache persistence remain performance work.
- The diagnostic mesher currently has a whole-world vertex cap. Prioritizing
  nearby chunks makes truncation predictable but does not turn incomplete
  geometry into full visual support. The client must disclose cap hits.
- A single global terrain revision can repeatedly invalidate a whole-world mesh
  while blocks keep changing, starving publication under sustained updates.
  Per-section jobs/revisions and bounded incremental publication are needed;
  Event Horizon's scheduler cannot fix work discarded before enqueueing.
- Frame count is currently fixed at two. A TOML request for one frame in flight
  needs a renderer configuration API before that setting is effective.
- GPU preference is currently automatic discrete-first selection, not a complete
  user-selection implementation. Nonstandard fallback surface formats have not
  been visually qualified.
- A successful colored-terrain screenshot does not validate vanilla rendering,
  movement, multiplayer compatibility, or the 240 FPS gameplay acceptance target.

## Verification recorded so far

`cargo check -p void-render`, renderer formatting, and strict renderer Clippy pass.
`vulkaninfo --summary` on the development laptop reported an NVIDIA GeForce RTX
5060 Laptop GPU with Vulkan 1.4.341 and an Intel integrated GPU with Vulkan 1.4.323.
The available instance-layer inventory did **not** include
`VK_LAYER_KHRONOS_validation`; therefore no validation-layer success is claimed.

Runtime image smoke tests, resize/minimize/restore sequences, repeated screenshots,
Linux Wayland/X11 runs, validation-layer runs, and hardware benchmark results must
be recorded separately by the main implementation task. Static review and builds
are not substitutes for those checks.
