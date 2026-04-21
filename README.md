# d2-little

Lightweight D2 → SVG converter: a pure-Rust port of the
[d2lang](https://d2lang.com) pipeline (parser → IR → graph → dagre
layout → SVG). Builds to native or `wasm32-unknown-unknown`, has no C
dependencies, and ships its own dagre layout engine so no JS bridge is
required.

## Status

`d2-little` targets byte-exact SVG parity with the upstream Go `d2`
reference for the "dagre + sketch" rendering configuration. The full
e2e corpus (several hundred cases from the Go test suite) is exercised
under `cargo test --workspace`.

## Rust

```rust
let svg_bytes: Vec<u8> = d2_little::d2_to_svg("a -> b")?;
```

The crate is a single-crate library: its public surface is the
top-level `d2_to_svg`, `parse`, `compile`, and `set_dimensions`
entry points, with sub-modules (`ast`, `ir`, `graph`, `compiler`,
`dagre_layout`, `svg_render`, etc.) exposed for consumers who want to
drive the pipeline phase-by-phase.

## Web / WebAssembly

The sibling crate `packages/web` is published to npm as
[`@kookyleo/d2-little-web`](https://www.npmjs.com/package/@kookyleo/d2-little-web).
It exposes the same `d2_to_svg` entry point as a wasm-bindgen
`convert(source: string) => string`:

```ts
import { convert } from '@kookyleo/d2-little-web';
const svg = convert('a -> b');
```

## Attribution

`src/dagre/` is a Rust port of
[dagre.js](https://github.com/dagrejs/dagre) (Apache-2.0). See
`src/dagre/LICENSE` and `src/dagre/README.md` for the upstream notices.

## License

Apache-2.0. See `LICENSE`.
