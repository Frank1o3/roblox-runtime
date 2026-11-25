# Licensing and source attribution

Project-owned runtime code is distributed under **GPL-3.0-or-later**. The
repository's `LICENSE` file contains the license text, and the workspace
package metadata uses the matching SPDX identifier.

This declaration resolves the former mismatch between the BSD-3-Clause root
license/manifest and the existing provenance notices, which identify covered
project code as GPL-3.0-or-later. It is based on the provenance records already
checked into this repository; it is not a legal opinion or a substitute for a
complete per-file copyright audit.

## Identified upstream adaptations

- **Eclipse:** The ELF parser, mapper, relocation decoder, and TLS layout
  foundations named in
  [`crates/linker/ECLIPSE-NOTICE.md`](../crates/linker/ECLIPSE-NOTICE.md)
  are adapted from Eclipse and retain its MIT attribution.
- **libjnivm:** The direct ByteBuffer JNI entries named in
  [`crates/jnivm/LIBJNIVM-NOTICE.md`](../crates/jnivm/LIBJNIVM-NOTICE.md)
  are adapted from libjnivm and retain its MIT attribution.
- **Apache-2.0-derived portions:** The porting inventory records Apache-2.0
  material identified by cordial's NOTICE. Preserve relevant file-level
  copyright and license notices whenever those portions remain in the tree.

These notices identify specific upstream-derived files; they do not remove or
replace the repository's GPL-3.0-or-later declaration for the remaining
project-owned runtime code.

## Ongoing provenance requirements

The source comparison in `docs/porting-plan.md` is historical and predates
some removals. When files are copied, adapted, replaced, or removed, update the
relevant notice and migration inventory. New ports should be implemented from
public specifications and measured behavior; do not copy implementation code
from reference projects without reviewing its license and preserving required
attribution.
