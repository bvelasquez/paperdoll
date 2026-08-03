# Motion files (`.vrma`)

Place [VRM Animation](https://vrm.dev/en/vrma/) files here, or use the built-in catalog:

```sh
paperdoll fetch-demo-motions          # download samples into this folder
paperdoll import-demo-motions         # fetch + convert to assets/animations/vrma_*.yaml
make demos                            # same as import-demo-motions
```

Single file:

```sh
paperdoll import-vrma assets/motions/Clapping.vrma --name vrma_clapping
curl -s -X POST http://127.0.0.1:7878/import/vrma \
  -H 'Content-Type: application/json' \
  -d '{"path":"motions/Clapping.vrma","name":"vrma_clapping","play":true}'
```

## Bundled catalog (via `fetch-demo-motions`)

| File | Animation name | Source |
|------|----------------|--------|
| `Clapping.vrma` | `vrma_clapping` | [vrm-viewer/VRMA](https://github.com/tk256ailab/vrm-viewer/tree/main/VRMA) |
| `Jump.vrma` | `vrma_jump` | same |
| `Goodbye.vrma` | `vrma_goodbye` | same |

Check the vrm-viewer repo for license terms before redistributing `.vrma` binaries.

After import, trigger playback: `POST /animation` with `{"name":"vrma_clapping"}` (restart the app if you imported YAML offline).
