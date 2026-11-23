# Solar installer artwork

`background.svg` is the editable source. The tracked exports are 700 × 500 and
1400 × 1000 pixels; dmgbuild combines them into a Retina background. The header
reuses Solar's existing app icon. Finder supplies the actual app and Applications
icons at (170, 250) and (530, 250), with 112-point icons. The light caption strips
keep Finder's own filename labels readable in both appearances.

To regenerate the exports with CairoSVG 2.9.1 and DejaVu Sans, from the repository
root (the trusted SVG references the local app icon):

```sh
cairosvg --unsafe scripts/ci/dmg/background.svg -o scripts/ci/dmg/background.png -s 1
cairosvg --unsafe scripts/ci/dmg/background.svg -o scripts/ci/dmg/background@2x.png -s 2
```

CI installs the hash-pinned packaging tools into a temporary Python environment
before signing. `create-dmg.py` writes Finder metadata without launching Finder,
then mounts the result read-only and verifies the app, artwork, icon positions,
and window settings. The same verification runs again after signing, notarizing,
and stapling, before publication.
