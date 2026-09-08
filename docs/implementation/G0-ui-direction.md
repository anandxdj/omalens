# G0 Omarchy UI direction

Status: implemented as a read-only readiness panel; live camera controls remain gated on the daemon contract.

## Design system

- Palette: use Omarchy's current foreground, accent, urgent, and surface colors. Camera state must fit every theme; the plugin does not introduce a competing brand palette.
- Type: use the shell's configured font and its existing type scale. Technical diagnostic output is compact but remains in the same family.
- Layout: a 440 px centered panel. A quiet 16:9 viewfinder is the dominant shape, with four crop-corner marks and one plain-language state. Readiness details sit below as a single aligned list.
- Interaction: the bar icon opens the panel; one refresh action reruns read-only checks. Disabled capture controls are not rendered.

```text
┌────────────────────────────────────┐
│  ┌─                            ─┐  │
│          Camera setup needed       │
│  └─                            ─┘  │
│                                    │
│  Host readiness              ↻     │
│  Omarchy                    Ready  │
│  Virtual camera             Missing│
│  ...                               │
└────────────────────────────────────┘
```

The crop corners evoke a phone viewfinder without decorative gradients or generic dashboard cards. The state text does the real work and never relies on the icon or color alone.

