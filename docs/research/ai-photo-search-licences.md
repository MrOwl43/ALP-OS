<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# AI photo search in the gallery: model licences (0.2.8 research)

Checked 2026-11-26 against the upstream pages listed below. Not legal
advice; a lawyer should confirm before any paid offer depends on it.

Immich's machine learning (ML) service is off in ALP-OS (ADR 0015) and its
image is not shipped. Turning it on brings two kinds of models, downloaded
by the ML service from Immich's Hugging Face account:

| Feature | Default model | Licence found | Can ALP-OS ship it? |
|---|---|---|---|
| Face recognition (people) | InsightFace `buffalo_l` | Code MIT, but the models are "available for non-commercial research purposes only"; since 2025-11 InsightFace asks to be contacted for licensing of `buffalo_l`; insightface.ai sells commercial model licences. The Immich copy is tagged `License: insightface`. | **No**, not without a licence bought from InsightFace. Keep faces off. |
| Smart search (type "beach", "dog") | CLIP `ViT-B-32__openai` | OpenAI's CLIP repository is MIT; the model card calls "any deployed use case" out of scope (a recommendation, not a licence term). The Immich card states no licence. | Unclear; avoid as the default. |
| Smart search, alternative | OpenCLIP models trained on LAION-2B (e.g. `ViT-B-32__laion2b_s34b_b79k`) | Model card licence field: `mit`; same out-of-scope wording; LAION does not recommend its data for "ready-to-go industrial products". | **Probably yes** (MIT), with the out-of-scope wording noted. Each multilingual model needs its own check (none shows a licence in Immich's collection). |

## Recommendation

1. Face recognition stays off in every ALP-OS build. Turning it on needs a
   commercial licence from InsightFace or a model with an open licence (a
   separate research item).
2. Smart search may come later with a LAION-trained OpenCLIP model whose
   card says MIT, pinned by digest like every other component, the ML image
   added through the usual component review (one-line justification, test
   run, memory use on a 4 GB machine). Not in 0.2.8.
3. Everything runs on the server; no photo leaves the home. Immich's own
   code (AGPL-3.0) is already in use.

## Sources

- InsightFace README, licence section:
  https://raw.githubusercontent.com/deepinsight/insightface/HEAD/README.md
- InsightFace licensing: https://insightface.ai
- Immich `buffalo_l`: https://huggingface.co/immich-app/buffalo_l
- Immich `ViT-B-32__openai`: https://huggingface.co/immich-app/ViT-B-32__openai/blob/main/README.md
- Immich multilingual CLIP collection: https://huggingface.co/collections/immich-app/multilingual-clip
- OpenAI CLIP: https://github.com/openai/CLIP and https://huggingface.co/openai/clip-vit-base-patch32
- LAION OpenCLIP ViT-B-32: https://huggingface.co/laion/CLIP-ViT-B-32-laion2B-s34B-b79K/raw/main/README.md
