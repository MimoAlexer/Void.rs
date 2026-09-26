# Java 26.2 protocol core

`void-protocol` is an independent Rust wire implementation. It contains no Azalea or
other Minecraft client/protocol runtime. It is an **incomplete gameplay subset**.
Successful status, a socket fixture, or decoded chunks do not establish full vanilla
or Paper compatibility.

## Verified version inputs

The official [26.2 version manifest](https://piston-meta.mojang.com/v1/packages/33c420747ce582e48dff1d8c5d8e67e5bb6257c9/26.2.json)
identifies the binaries used for local read-only inspection:

| Input | SHA-1 |
| --- | --- |
| Mojang server bundler | `823e2250d24b3ddac457a60c92a6a941943fcd6a` |
| Mojang client | `2dc72797acbc1b63fc16a11c4ac393605f453754` |

Their `version.json` identifies **protocol 776**, **data version 4903**, Java 25,
resource pack 88.0, and data pack 107.1. The official data-generator `--reports`
output supplies every packet name/ID in `data/protocol-26.2.toml`.

Client `net.minecraft.client.Options` bytecode verifies render-distance **2..32**,
default **12**. Vanilla restricts the maximum to 16 when its JVM maximum memory is
below 1,000,000,000 bytes; Void.rs uses the ordinary 2..32 range.

Key wire differences were inspected in official classes, including:

- Login success includes the profile followed by a session UUID.
- Play login includes `onlineMode` and `enforcesSecureChat` after spawn information.
- Chunk sections include both non-air block count and fluid count.
- Paletted containers write fixed-length packed long arrays **without a length
  prefix**, including no array length after a single-valued palette.
- Compact entity velocities use `LpVec3`, verified with official golden bytes.
- Section block updates use VarLong records; contemporary unofficial schemas can
  incorrectly describe them as VarInts.
- Entity teleport contains PositionMoveRotation, relatives and on-ground; it is
  not the older x/y/z + byte-rotation layout.
- Sprint commands use action ordinal 1/2 in 26.2, not the old 3/4 values.

## API and behavior

`ServerAddress::parse`, `status(address, timeout)`, `ConnectOptions::offline`,
`connect_offline`, and `connect_online(options, OnlineIdentity)` expose network
operations. Connecting spawns a named networking thread; UI/render callers consume
bounded event and command channels without socket calls. `status` is synchronous
and must also run off the render thread.

Implemented paths include ordinary hostname/IP status and ping; handshake;
login, compression, RSA/AES-CFB8 negotiation and HTTPS session join; configuration
acknowledgements; dimension-registry NBT; empty known-pack negotiation (requesting
complete registry data); configuration/play keepalives and pings; world entry;
section palette decoding; teleport acknowledgement; movement; entity spawn,
position, removal and velocity; single/section block updates; health/abilities;
unsigned outgoing chat and commands; system chat; sprint, swing and attack packets.

`OnlineIdentity` holds an in-memory access token whose Debug output is redacted
and whose backing string is zeroed on drop. Tokens never enter TOML, packet logs,
or process arguments. The session join endpoint uses HTTPS and does not follow
redirects. The online account exchange lives in `void-services`. A real Microsoft
account/server session still requires a valid app registration and live testing.

The codec caps wire frames at 2,097,151 bytes, decompressed packets at 8 MiB,
network NBT depth at 64, and NBT collections/nodes at a bounded budget. Compression
thresholds, declared lengths, UTF encodings and partial socket reads are checked.
Events have 256 slots and commands 128; a non-consuming client disconnects with a
backpressure error instead of accumulating unlimited memory. A complete connection
read remains subject to the OS resolver's own DNS timeout.

Server conduct text requires an explicit `AcceptCodeOfConduct` command. This core
does not silently accept it. Resource packs are explicitly declined; transfers
return an unsupported-requirement diagnostic. Unknown required login/configuration
packets fail; unimplemented play packet kinds produce one notice each.

## Generated block data

`block_states::info(state)` identifies all **32,366** official states and 1,196
blocks, including cave/void air. `collision_boxes(state)` returns block-local
minXYZ/maxXYZ boxes from 326 deduplicated vanilla shapes.
`physics(state)` returns `[friction, speed_factor, jump_factor]`; `friction(state)`
is a convenience accessor. These are factual compatibility tables, not textures,
models, audio, jars, decompiled source, or other redistributed game assets.

Shapes use `EmptyBlockGetter` and the empty entity context. Entity-dependent
collision, scaffolding, powder snow, world-dependent checks and other behavior
still need client simulation; static boxes alone do not implement those mechanics.

`examples/GenerateBlockStates.java` bootstraps vanilla registries and exports TSV.
`examples/PackBlockStates.ps1 -InputTsv <path>` packs it into Rust tables/binary
indexes. `examples/GenerateProtocolFixtures.java <output-directory>` emits golden
chunk-palette and compact-velocity bytes using the official codecs.

To reproduce, obtain the version-matched official jars into ignored `.local/`,
verify the hashes above, and run Java 25 with a classpath containing the extracted
server jar and the bundler's dependency jars. The source-file launcher runs each
Java generator directly. The official reports are generated by:

```text
java -DbundlerMainClass=net.minecraft.data.Main -jar server-26.2.jar --reports --output generated-26.2
```

Run that command with its working directory inside `.local/`: the bundler extracts
libraries/versions beside the process. It invokes a data generator, not a server,
and does not accept a server EULA. Run `cargo fmt -p void-protocol` after packing.

## Validation and remaining work

Run `cargo test -p void-protocol` and `cargo clippy -p void-protocol --all-targets -- -D warnings`.
Tests include fragmented/compressed frames, malformed limits, NBT nesting and
modified UTF-8, NIST AES-CFB8 bytes, signed Minecraft hashes, official golden
palettes/velocities, generated-state lookups, and a socket session spanning
compression, configuration, terrain, teleport, movement, and keepalives.

For renderer integration only:

```text
cargo run -p void-protocol --example fixture_server -- 127.0.0.1:25566
```

This deterministic fixture serves nine chunks and one moving entity over actual
26.2 socket frames. It accepts movement and attack messages but does not simulate
authoritative gameplay. It labels itself as a test fixture in status and chat.

Still incomplete: DNS SRV, full dimension/resource/registry consumption, block
entity and light application, inventory/item/component codecs, player-list/skin
state, signed chat/last-seen tracking, all entity metadata/attributes/effects,
server GUI/screens, sounds/particles, recipes, packs, transfer/cookie persistence,
bundle-atomic state publication, and complete vanilla physics/combat prediction.
The protocol sends attack/swing packets; that does not implement all combat rules.
Microsoft online-server joins, real vanilla/Paper sessions, Linux behavior and
long-duration network impairment tests have not yet been verified.
