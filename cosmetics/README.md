# cosmetics

World of Kartcraft's own cosmetic models, sold for Kartcraft Tokens in the shop (`/kartshop`)
and built into the client (`crates/kartcraft/src/goods.rs`), so every player has every one and
sees the ones others wear.

These are art, not code: they are **not** covered by the repository's MIT OR Apache-2.0
licence. Each file is a gzipped JSON in the kart data's format (`docs/ctr-assets.md`): a whole
kart (`kart-<key>.json.gz`, the `ctr-kart` format with anchors, wheels and surfaces) or a flag
(`flag-<key>.json.gz`, textures and surfaces only, Y up, meters, the pole's foot at the origin,
the cloth along +X).

| File | Item | Triangles |
|---|---|---|
| `kart-fel.json.gz` | Fel kart | 17,117 |
| `kart-alliance.json.gz` | Alliance kart | 1,541 |
| `kart-wok.json.gz` | WoK kart | 80,670 |
| `flag-student.json.gz` | Student flag | 1,548 |

The sources (`.blend`, `.glb`, previews) stay with their authors.
