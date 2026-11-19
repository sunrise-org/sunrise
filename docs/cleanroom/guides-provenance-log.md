# Provenance log: built-in mod guides specification, revision 2

What the spec team read to write [guides-spec.md](guides-spec.md), and nothing else. Section 13 of the spec maps each
section to these sources. Feature request: sunrise-org/sunrise#2664.

Session: one AI agent (Claude Opus 5.5), 2026-11-18, on the maintainer's machine, working in the `mod-guides` worktree of
Sunrise at `74a14752ad8c13636d852d61ed809d0724880df9`. Downloads went to a scratch folder on the same drive, outside
every repository and outside the quarantine; they are not kept here. Every fetched documentation page was first checked
by a script that printed only its hash, the number of preformatted blocks, their highlighting languages and counts of
code markers (`` ```java ``, `public class`, `import `, `@Override`, `gradle`, `kotlin`, `.java`), never page text.

## 0. Revision 2 replaces a discarded revision 1

This is **revision 2**. A first attempt (revision 1) was discarded because its author read Java code examples on
GuideME's documentation site. This session did not open, list or read that attempt or anything in the folder it was
moved to, and started from the issue, `CONTRIBUTING.md` and `docs/cleanroom/README.md` only. No text of revision 1 is
in this revision.

## 1. Documentation pages read (GuideME's documentation site)

Site: https://guideme.appliedenergistics.org/ (fetched 2026-11-18 between 09:10 and 09:15 UTC with `curl`, saved as HTML,
converted to text locally). The site shows no revision; the most recent commit touching its documentation folder at
fetch time was `817a15899192286954f5d1078e34af0db2db9815` (2026-11-13T20:36:41Z), taken from the GitHub commits API
(commit metadata only; no file of that repository was opened).

| Page | Code check | Read | SHA-256 of the fetched HTML |
|---|---|---|---|
| `/sitemap.xml` (list of page URLs) | n/a | URLs only | `1b5d23d658483a6b202188bd6096473d617a791762e97200830de3d769664ce7` |
| `/` Overview | one block, page markup (Markdown with tags); no code markers | yes | `62108f0d1da58066c88bb1f4ba5c42b22fb3bf4a4e6596d011c09a88e4cf07f2` |
| `/authoring/` Authoring Pages | blocks of Markdown, YAML and text; no code markers | yes | `037214a32530ae5ed0b09f265d6c24d8ff29d9b642618a8da7ff0251f6a86446` |
| `/authoring/markdown` Supported Markdown | Markdown and text blocks; no code markers | yes | `95fbd13b38e4a969148f170e353f400b01ac233f7985dbcaa4b1cc46b7df143d` |
| `/authoring/game-scenes` Game Scenes | 13 blocks highlighted as text/jsx; the first 70 characters of each were checked: all are scene tag markup; no code markers | yes | `0dee6626c58441064a1a7a61f5ca2b2a800f75023f481df91a033a194ae125c6` |
| `/data-driven-guides` | two JSON blocks and one `/give` command; first 70 characters checked; no code markers | yes | `de99d3d8dbae94a458058ae446671334bb1f67a8f55f19dc5ef0c582d0a2b2d9` |
| `/commands` | no blocks | yes | `5bc71e3cd25915d6b3b9c7cdace616559b03f63113f17357a1995d33645fe983` |
| `/open-guide-hotkey` | no blocks | yes | `dcef3b7a595019bf57fa4fb6dc7c7a25b720d20ee74901d013241b11670a9972` |
| `/search` | no blocks | yes | `b43f60e86f055aab9df14b0d8034c4e9541b605446e9ed184261061e58feea56` |
| `/translation` (second fetch; the first returned a GitHub Pages error page) | no blocks | yes | `1458db3173e95f67121d0ed95ded1ec279b5af9f2d3ad12a02f268b15981e59f` |
| `/license` | no blocks | yes (licence facts only) | `ff08e4aa7d8661974bc12bfa93c3eaca9db1b182ddf32032ceafd200d1647b93` |
| `/converting-patchouli` | no blocks; an upload widget and a video | yes (nothing used) | `76faf8dd99e6df5695c009f20aa4db21635b6004c8c6f8e043f956252b21b315` |

## 2. Pages skipped

| Page | Why |
|---|---|
| `/integration/`, `/integration/custom-tag`, `/integration/recipe-types`, `/integration/symbolic-colors`, `/integration/website-export` | Integration pages: forbidden by the brief. Never fetched. |
| `/javadoc/` | API documentation. Never fetched. |
| `/downloads` | Fetched by the batch download; the code check found Gradle blocks. Not converted, not read. (`d23a82a3…7e01`) |
| `/live-preview` | Fetched by the batch download; the code check found Groovy/Gradle blocks. Not converted, not read. (`5c5114aa…a50db`) |
| `/changelog` | Developer-facing release notes likely to name code. Fetched by the batch download; not converted, not read (see incident 2). (`01e70e7d…472d`) |
| The overview's "source" link to its example page on GitHub, the "Edit this page" links, the link to GuideME's test mod, the GitHub repository | Lead to repositories with code. Never opened. |

## 3. Guide page files read as data

From two published jars of Applied Energistics 2 on Modrinth, downloaded to the scratch folder. Only entries under
`assets/ae2/ae2guide/` ending in `.md`, `.snbt` or `.json` were extracted; no class file, no other entry was extracted
or opened. The page files were analysed mostly by a script that tabulated front matter keys, tag names, attribute
names, link and image target forms and SNBT top-level keys; a few short excerpts were looked at directly (the front
matter of four pages, a handful of `GameScene` lines, four heading lines, and the smallest `.snbt` file). No page prose
is reproduced in the spec.

| Jar | Source URL | SHA-256 | Extracted |
|---|---|---|---|
| `appliedenergistics2-26.3.2-alpha.jar` (Minecraft 26.3, NeoForge; Modrinth SHA-1 `76fa7a2b4fed12ac1e008e4e78b49ce5b3f73b58`) | https://cdn.modrinth.com/data/XxWD5pD3/versions/KuBWeko9/appliedenergistics2-26.3.2-alpha.jar | `ff39ecf90fbce1aff575e9700207e6efd977ef8a222edd4bb0bcc1398b02e54d` | 248 files (125 `.md`, 123 `.snbt`) |
| `appliedenergistics2-forge-15.4.12.jar` (Minecraft 1.20.1, Forge) | https://cdn.modrinth.com/data/XxWD5pD3/versions/QRpVsqDI/appliedenergistics2-forge-15.4.12.jar | `b9c5928893a20d3feb0b4393e2c23d756a16d9abee24d9c14c98e6096d53844d` | 236 files |

Licence note: `CONTRIBUTING.md` allows reading file formats and user documentation whatever the licence, and the
issue names "guides' own page files as a file format" as a source. The pages were used for format facts only (which
keys, tags, attributes and file layouts exist), not for their content.

## 4. Other sources

| Source | How | Revision | SHA-256 |
|---|---|---|---|
| Minecraft Wiki, "Structure file", section "NBT structure" | `https://minecraft.wiki/w/Structure_file?action=raw`, 2026-11-18 | as of fetch | `d054471ebf7c144e76a3942ee2845fc73bf5e28f95606fd22652ceb0c31b4d31` |
| Minecraft Wiki, "Argument types", section `minecraft:item_stack` | `https://minecraft.wiki/w/Argument_types?action=raw`, 2026-11-18 | as of fetch | `f1c4c0d39525d1beb69417ccda2132b7d0fcd676067c287fe6eb18bb51e4b9cc` |
| sunrise-org/sunrise#2664 (issue body) | GitHub REST API, 2026-11-18 | `updated_at` 2026-11-18T09:03:28Z | not kept |
| Modrinth version listings for AE2 (version numbers, file names, URLs) | Modrinth API v2 | as of fetch | not kept |
| Sunrise worktree: `CONTRIBUTING.md` (section "Only original code…"), `docs/cleanroom/README.md`, `docs/cleanroom/provenance-log.md` (format), `docs/modding/src/how-to/client/living-scenes.md`, `docs/modding/src/how-to/items/panel-style.md`, `docs/extras.md` (first 40 lines), the module comment and first 60 lines of `crates/sunrise-client/src/game/modding/native_scene_live.rs`, grep hits for key and panel names in `crates/sunrise-client/src/game/` | local | `74a14752ad8c13636d852d61ed809d0724880df9` | the commit pins them |
| Minecraft 26.3 behaviour | general knowledge of the game, no files | 26.3 | n/a |

No web search was used.

## 5. Deliberately not opened

- Any source code of GuideME, AE2 (including its guidebook package), Patchouli, Modonomicon or any other guide mod, on
  GitHub or in jars; any class file.
- The quarantine folder holding revision 1, and everything else on the build drive outside the `sunrise-guides`
  worktree.
- The port workspace and its files, and session transcripts or memory files.

## 6. Incidents and caveats

Incidents: none in which code was read.

1. **Jar listing showed names.** To find the guide folder, a script listed the entry names of the 26.3 jar that contain
   "guide". The list included three service-registration file names under `META-INF/services/` and one package
   directory name of the mod's guide code. These are names only; nothing behind them was opened, and none of them is
   used in the spec. They are not repeated here so the implementation team never sees them (the name tripwire of
   [README.md](README.md) applies to them).
2. **Changelog fragment.** While checking why the code check counted the word `void ` on the overview and changelog
   pages, a grep printed 30 characters on each side of the match. On the overview it was the site's page-loader script;
   on the changelog a fragment of prose ("…avoid constantly changing block appe…", a match inside "avoid"). No code;
   nothing used. The changelog was not otherwise read.
3. **Downloads and live-preview HTML were fetched** in a batch before the code check (the check runs on the saved file).
   They were not converted to text or read after the check flagged build-script blocks.
4. **Session context.** The session was started from a parent agent in a different working directory; its automatically
   loaded context contained a memory index of one-line notes and a `git status` listing of file names of an unrelated
   local workspace. Neither contains guide code. One note states the user's wish that a guide open with W, which the
   issue states anyway. Nothing from that context is used in the spec beyond the issue's own words.
5. **Model training.** The spec writer is a language model whose training data may include public source code of the
   projects above. In this session no such code was consulted, and every statement of the spec is tied to a source in
   sections 1, 3 or 4 or marked as a Sunrise design choice. If that is considered disqualifying, the spec can be
   re-derived by a reviewer from the same sources: the tags make every statement checkable.
6. **Documentation inconsistencies recorded, not resolved:** the Game Scenes page names the removal tag both
   `RemoveBlock` and `RemoveBlocks` and its example contradicts its prose about barriers (spec Q-035); the `interactive`
   scene attribute is used in examples but missing from the attribute table (spec 6.1); the symbolic colour table's
   values are swatches, not text (Q-025).
