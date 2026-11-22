-- The lab's MIDI ports, its targets, and this extension's surfaces.
return {
    ports = host.ports("midi"),
    targets = host.targets(),
    surfaces = host.surfaces(),
}
