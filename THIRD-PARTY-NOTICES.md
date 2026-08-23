# Third-party notices

`@voxelkloud/wasm-codecs` ships a compiled wasm module. Unlike the rest of this
repo, that binary statically links third-party Rust code, so the notices below
travel with the published package.

`voxelkloud_wasm_codecs_bg.wasm` links, and only links:

| Crate | Version | License |
| --- | --- | --- |
| [laz](https://github.com/tmontaigu/laz-rs) | 0.13.0 | Apache-2.0 |
| [byteorder](https://github.com/BurntSushi/byteorder) | 1.5.0 | MIT OR Unlicense |
| [num-traits](https://github.com/rust-num/num-traits) | 0.2.19 | MIT OR Apache-2.0 |
| [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) | 0.2.127 | MIT OR Apache-2.0 |
| [cfg-if](https://github.com/rust-lang/cfg-if) | 1.0.4 | MIT OR Apache-2.0 |
| [once_cell](https://github.com/matklad/once_cell) | 1.21.4 | MIT OR Apache-2.0 |

Every one but `laz` is available under MIT and taken under it. `laz` is
Apache-2.0 only, which is the one notice that has to be reproduced.

None of these are modified. `laz` is used as published on crates.io; this
package adds a wasm surface around it and reimplements only the small dispatch
that builds a record decompressor from laz items, because upstream keeps its
own copy private behind a file-shaped API that requires a chunk table.

## laz (laz-rs)

The Rust port of Martin Isenburg's LASzip, by Thomas Montaigu.

Upstream: https://github.com/tmontaigu/laz-rs

```
Copyright [2023] [Thomas Montaigu]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

PROGRAMMERS:

  martin@rapidlasso.com (For the original c++ LASzip)
  thomas.montaigu@laposte.net (Rust port)
```

A full copy of the Apache License, Version 2.0 is at
http://www.apache.org/licenses/LICENSE-2.0.
