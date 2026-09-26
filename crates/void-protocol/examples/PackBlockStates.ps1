param(
    [Parameter(Mandatory=$true)][string]$InputTsv,
    [string]$Destination = (Join-Path $PSScriptRoot '../src')
)
$ErrorActionPreference = 'Stop'
$destinationPath = (Resolve-Path -LiteralPath $Destination).Path
$rows = Import-Csv -LiteralPath $InputTsv -Delimiter "`t"
if ($rows.Count -ne 32366) { throw 'Expected the verified 26.2 registry (32366 states).' }
$flags = [byte[]]::new($rows.Count)
$collisions = [byte[]]::new($rows.Count * 2)
$physics = [byte[]]::new($rows.Count)
$shapeIds = [Collections.Generic.Dictionary[string,int]]::new()
$shapes = [Collections.Generic.List[string]]::new()
$physicsIds = [Collections.Generic.Dictionary[string,int]]::new()
$triples = [Collections.Generic.List[string]]::new()
foreach ($row in $rows) {
    $id = [int]$row.state_id
    $f = 0
    if ($row.air -eq 'true') { $f = $f -bor 1 }
    if ($row.full_collision -eq 'true') { $f = $f -bor 2 }
    if ($row.can_occlude -eq 'true') { $f = $f -bor 4 }
    if ($row.fluid -eq 'true') { $f = $f -bor 8 }
    $flags[$id] = [byte]$f
    $shape = [string]$row.boxes
    if (!$shapeIds.ContainsKey($shape)) { $shapeIds[$shape] = $shapes.Count; $shapes.Add('    &[' + $shape + '],') }
    $shapeId = $shapeIds[$shape]
    if ($shapeId -gt 65535) { throw 'Shape index no longer fits u16.' }
    $collisions[$id*2] = [byte]($shapeId -band 255)
    $collisions[$id*2+1] = [byte]($shapeId -shr 8)
    $triple = '[' + $row.friction + ',' + $row.speed_factor + ',' + $row.jump_factor + ']'
    if (!$physicsIds.ContainsKey($triple)) { $physicsIds[$triple] = $triples.Count; $triples.Add($triple) }
    if ($physicsIds[$triple] -gt 255) { throw 'Physics index no longer fits u8.' }
    $physics[$id] = [byte]$physicsIds[$triple]
}
$definitions = foreach ($group in ($rows | Group-Object block)) {
    $ids = $group.Group | ForEach-Object { [int]$_.state_id }
    $bounds = $ids | Measure-Object -Minimum -Maximum
    if ($bounds.Maximum - $bounds.Minimum + 1 -ne $ids.Count) { throw 'Non-contiguous block state IDs.' }
    '    ({0}, {1}, "{2}"),' -f $bounds.Minimum,$bounds.Maximum,$group.Name
}
$definitions = $definitions | Sort-Object { [int]([regex]::Match($_,'\d+').Value) }
$header = @'
//! Generated factual state IDs and classification from Mojang 26.2, not assets.
//! Regenerate with GenerateBlockStates.java and PackBlockStates.ps1; see docs/PROTOCOL.md.
const FLAGS: &[u8] = include_bytes!("block_flags.bin");
#[derive(Debug,Clone,Copy)]
pub struct BlockInfo { pub name: &'static str, pub air: bool, pub full_collision: bool, pub can_occlude: bool, pub fluid: bool }
pub fn info(state: u32) -> Option<BlockInfo> {
    let flags = *FLAGS.get(state as usize)?;
    let i = BLOCKS.partition_point(|(start,_,_)| *start <= state).checked_sub(1)?;
    let (start,end,name) = BLOCKS[i];
    if state < start || state > end { return None; }
    Some(BlockInfo { name, air: flags&1!=0, full_collision: flags&2!=0, can_occlude: flags&4!=0, fluid: flags&8!=0 })
}
pub const STATE_COUNT: usize = FLAGS.len();
const BLOCKS: &[(u32,u32,&str)] = &[
'@
$collisionCode = @'
/// Static-context vanilla collision boxes (min X/Y/Z, max X/Y/Z), in block-local units.
/// Conditional shapes requiring entity context (e.g. powder snow and scaffolding) still
/// require gameplay rules; this table uses vanilla EmptyBlockGetter, empty context.
pub fn collision_boxes(state: u32) -> &'static [[f32;6]] {
    let start = state as usize * 2;
    let Some(bytes) = COLLISION_INDICES.get(start..start+2) else { return &[]; };
    COLLISION_SHAPES[u16::from_le_bytes([bytes[0],bytes[1]]) as usize]
}
const COLLISION_INDICES: &[u8] = include_bytes!("collision_indices.bin");
const COLLISION_SHAPES: &[&[[f32;6]]] = &[
'@
$physicsCode = @'
/// Vanilla static block friction, speed factor, and jump factor respectively.
pub fn physics(state: u32) -> [f32;3] {
    let Some(index) = PHYSICS_INDICES.get(state as usize) else { return [0.6,1.0,1.0]; };
    PHYSICS[*index as usize]
}
pub fn friction(state: u32) -> f32 { physics(state)[0] }
const PHYSICS_INDICES: &[u8] = include_bytes!("physics_indices.bin");
'@
$source = $header + "`n" + ($definitions -join "`n") + "`n];`n" + $collisionCode + "`n" + ($shapes -join "`n") + "`n];`n" + $physicsCode + "`nconst PHYSICS: &[[f32;3]] = &[" + ($triples -join ',') + "];`n"
[IO.File]::WriteAllText((Join-Path $destinationPath 'block_states.rs'),$source,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllBytes((Join-Path $destinationPath 'block_flags.bin'),$flags)
[IO.File]::WriteAllBytes((Join-Path $destinationPath 'collision_indices.bin'),$collisions)
[IO.File]::WriteAllBytes((Join-Path $destinationPath 'physics_indices.bin'),$physics)
Write-Output "Packed $($rows.Count) states, $($shapes.Count) shapes, $($triples.Count) physics triples. Run cargo fmt -p void-protocol."
