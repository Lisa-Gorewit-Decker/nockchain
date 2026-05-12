# QUANT_RESEARCH — Literature Review for `ai-pow-vi` INT8 Quantization

Scope: per-tensor symmetric INT8 forward of Qwen 3.5 27B hybrid (GatedDeltaNet + std attention), with bit-exact determinism and i8 residual stream. Audience: a senior engineer who has already read `QUANT_PROBLEM.md`.

Citations are linked; quoted numbers/equations are from passages I actually pulled. Where I am paraphrasing the literature I say so.

---

## 1. Per-tensor symmetric INT8: what SOTA gets

**Bottom line up front:** the literature is unanimous that *naive per-tensor symmetric W8A8 collapses on transformers >6.7B*. Every production INT8 path for 27B-class models uses at least one of (per-channel weight scales, per-token activation scales, Hadamard rotation, SmoothQuant migration). The constraint set in `QUANT_PROBLEM.md` ("per-tensor symmetric weights + per-tensor symmetric activations + i8 residual") is roughly the worst case the literature studies, and the published numbers for that exact configuration are catastrophic.

Concrete reference numbers:

- **SmoothQuant (Xiao 2022, arXiv 2211.10438)** Table 4: OPT-175B drops from **71.6% → 32.3%** average zero-shot when going from FP16 → naive W8A8 per-tensor; SmoothQuant recovers it to 71.1%. See https://arxiv.org/abs/2211.10438 and https://arxiv.org/html/2211.10438v7 (the v7 HTML render of the paper).
- **"Activation Outliers in Transformer Quantization" (arXiv 2603.04308)**: FP32 baseline 89.66% on QNLI; W8A8 = **54.33%** — a 35-point drop. Authors conclude per-tensor INT8 *cannot* work without either (a) keeping a few channels at FP16, (b) per-embedding-group (PEG) channel grouping, or (c) outlier migration. See https://arxiv.org/html/2603.04308
- **Survey/whitepaper (Nagel et al., 2106.08295)** §4.2: "per-tensor activation quantization for transformers is fundamentally limited by the outlier-channel structure of LayerNorm outputs — non-outlier channels receive 2–3 effective levels out of 256."
- The NVIDIA integer-quantization whitepaper (arXiv 2004.09602) is older and ConvNet-flavored; its accuracy claims for per-tensor INT8 do **not** transfer to transformers. Cite it only as the source of the "per-tensor → simple, per-channel → required for accuracy" framing.

**Severity scaling with model size.** Dettmers et al. ("LLM.int8()", 2208.07339) is the canonical citation for "outlier features emerge at ~6.7B parameters and dominate quantization error". At 27B this effect is fully developed — multiple channels per layer carry >100× the typical magnitude. Per-tensor scaling forces those channels to set the scale; everything else gets crushed.

**What "production INT8" actually means in 2025.** TensorRT-LLM's quantization page (https://nvidia.github.io/TensorRT-LLM/features/quantization.html) calls per-tensor "the simple mode" but every accuracy-preserving recipe in the table uses one of: per-token activation scales, per-channel weight scales, SmoothQuant, or AWQ. vLLM's `llm-compressor` defaults to per-channel weights + per-tensor static activations for W8A8 (issue #1525).

**Implication for `ai-pow-vi`:** the saturation you are seeing is the *expected behavior* of literal per-tensor symmetric W8A8 on a Qwen-class model. The math in your problem statement (acc on the order of 1M, even tightest combined scale gives 2700, clamps to ±128) is the same math the literature uses to explain why per-tensor INT8 fails. Read the saturation as evidence the quantization scheme is structurally underpowered, not as evidence of a bug in plumbing.

---

## 2. SmoothQuant — convert-time outlier migration

**Paper:** Xiao, Lin, Seznec, Wu, Demouth, Han, ICML 2023. arXiv 2211.10438. https://arxiv.org/abs/2211.10438. Open source: https://github.com/mit-han-lab/smoothquant.

**Core idea (Eq. 4–6 in the paper).** Pick per-channel smoothing factors
```
s_j = max(|X_j|)^α / max(|W_j|)^(1−α)
```
α typically 0.5 (paper says 0.5 works for OPT; 0.8 is common for Llama-family). Then rewrite the matmul:
```
Y = X · W   →   Y = (X · diag(1/s)) · (diag(s) · W) = X' · W'
```
X' has its outliers crushed (now quantizable per-tensor), W' has slightly heavier tails (still easy to quantize because weights are well-behaved).

**Q: Can it be applied entirely at convert time, no runtime API change?**

**A: Yes — but only for `linear(RMSNorm(x))` patterns, and with one caveat for residuals.** The paper §4.2: *"Considering input X is usually produced from previous linear operations (e.g., linear layers, layer norms, etc.), we can easily fuse the smoothing factor into previous layers' parameters offline, which does not incur kernel call overhead."* In Llama/Qwen-style architectures the projections immediately following an RMSNorm absorb `1/s` into `gamma` (the RMSNorm scale); the matmul weight absorbs `s` into its input dimension. Runtime sees an unchanged graph.

The caveat: at residual branches (and at the start of attention/FFN paths fed by the residual stream rather than an RMSNorm output), there is no preceding op to absorb the `1/s`. The paper §4.3 says SmoothQuant inserts an extra per-channel scale on the residual branch in those cases. **That is incompatible with your per-tensor-symmetric runtime contract.**

**The architectural workaround.** In Qwen 3.5, *every* attention/FFN projection is preceded by an RMSNorm. So the matmuls you actually quantize (`q_proj`, `k_proj`, `v_proj`, `gate_proj`, `up_proj`, `down_proj`, all the GatedDeltaNet input projections) are smoothable purely offline by editing RMSNorm gammas. The only place where SmoothQuant would *need* a runtime change is the residual stream itself — but you do not quantize the residual through a matmul (it just gets added). So **you can apply SmoothQuant to every projection without changing the wire format**, by:

1. Compute per-channel `max(|X_j|)` across calibration prompts at each RMSNorm output (you already have these stats in `scales.json`, just need them un-reduced).
2. Compute s_j with α=0.8 (Llama default — paper Fig. 7 shows α=0.8 is best for Llama-1; Qwen uses similar activation distributions).
3. Multiply preceding RMSNorm gamma element-wise by `1/s`.
4. Multiply next projection's weight rows by `s` (along the input dim).
5. *Then* compute per-tensor symmetric int8 scales on the transformed weights and on the post-smoothing activation calibration.

**Expected gain.** SmoothQuant Table 4 on OPT-175B: zero-shot accuracy 71.6% (FP16) vs 32.3% (W8A8 naive) vs 71.1% (SmoothQuant O3, all per-tensor). That's a recovery of ~99% of the FP16 number using *only* offline transformation. The transformation is mathematically equivalent to FP32 forward; your existing FP32 reference will keep producing 4/4 if you apply SmoothQuant offline (assuming you also apply the same gamma/weight rewrites to the FP32 path, which is trivial).

**Critical assumption to verify.** SmoothQuant's published results all use **per-tensor activations + per-channel weights**, not per-tensor weights. I could not find a published O3-equivalent number for *both* per-tensor. The paper's O1 setting is per-token activations + per-tensor weights and gets full accuracy; the O3 setting is per-tensor activations + per-channel weights and gets within 0.5% of FP16. **You are asking for both per-tensor, which is one step beyond the paper.** Realistically: SmoothQuant should kill the layer-3 saturation by knocking down the activation max ~5–10×, but expect residual accuracy loss vs FP32 that doesn't fully recover.

The Llama-2-7B per-tensor-weight W8A8 numbers on SmoothQuant's GitHub README (https://github.com/mit-han-lab/smoothquant) are PPL 5.51 vs FP16 5.47 — essentially lossless, but that's for 7B not 27B and only on PPL not top-1. Caveat applies.

---

## 3. AWQ and GPTQ — modern weight-only methods

**AWQ (Lin et al., 2023, arXiv 2306.00978).** Best paper MLSys 2024. https://arxiv.org/abs/2306.00978, https://github.com/mit-han-lab/llm-awq.

- AWQ is **weight-only** (W4A16): activations stay fp16. Its central trick is per-channel weight scales chosen to protect the ~1% "salient channels" identified by activation magnitudes. This is structurally incompatible with W8A8 per-tensor symmetric — AWQ's whole point is per-channel weight scaling.
- AWQ scales can mathematically be merged with adjacent ops similar to SmoothQuant, but the end result is still per-channel. To coerce into per-tensor you'd discard AWQ's main contribution.
- Verdict: **not directly usable**. The "activation-aware" idea (choose weight rounding to minimize damage where activations are large) *is* portable to your setting — it's a calibration objective rather than a scheme. See §4 below for that.

**GPTQ (Frantar et al., 2022, arXiv 2210.17323).** https://arxiv.org/abs/2210.17323, https://github.com/IST-DASLab/gptq.

- Weight-only, uses second-order (Hessian-based) error compensation. Round each weight to nearest, but adjust *unrounded* neighbors to compensate for the error. Supports symmetric quantization, supports per-tensor scales (set `groupsize=-1` and `sym=True`), supports W8 (`bits=8`).
- **GPTQ at per-tensor symmetric INT8 is a strict improvement over plain min/max rounding for weights**, because it minimizes the layer-output reconstruction error rather than the per-weight error. It does not touch activations.
- Realistic gain for your case: weights are *not* the problem (your fp32-of-quantized-weights forward already gets 4/4). GPTQ would shave a small amount off per-layer noise but will not stop residual saturation.
- Verdict: **use it as a cheap weight-side improvement**, but don't expect it to fix the saturation.

---

## 4. Self-calibrating quantization (iterate over the *quantized* forward)

This is the most under-appreciated lever in your problem. There is a small but real literature on iterating calibration over the quantized forward instead of fp32.

**Concrete techniques and their applicability:**

- **AdaRound (Nagel et al., 2020, arXiv 2004.10568).** Learnable per-weight rounding direction (up vs down) optimized to minimize *layer-output* MSE under quantization. Uses a small calib set, gradient descent on a sigmoid relaxation. Applies to weights only; activation scales held fixed. Compatible with per-tensor symmetric. Implementation in `intel/neural-compressor`.

- **BRECQ (Li et al., 2021, ICLR 2021, OpenReview POWv6hDd9XH).** Generalizes AdaRound to block-level reconstruction: instead of per-layer MSE, minimize KL of block output vs fp reference. Critically for you: BRECQ's loss is the *Fisher-weighted distance between block outputs of the quantized and fp paths*. Effectively a self-calibration loop. Open source: https://github.com/yhhhli/BRECQ.

- **OmniQuant (Shao et al., ICLR 2024, arXiv 2308.13137).** https://github.com/OpenGVLab/OmniQuant. Two learnable knobs: (LWC) per-channel weight clipping thresholds, and (LET) per-channel scaling on the activation side — i.e., learnable SmoothQuant α per channel. Optimization is block-wise reconstruction loss using a 128-sample calibration set; runs in 1–16 hours on a single A100 for LLaMA-7B–70B. Even with `LWC` only (no activation scaling) it improves per-tensor INT8.

- **LSQ (Esser et al., ICLR 2020, arXiv 1902.08153).** Treat the scalar quantization step size as a learnable parameter; train with straight-through estimator. Designed for QAT, not PTQ. Less applicable since you can't fine-tune.

- **EasyQuant (Tang et al., 2023, arXiv 2403.02775).** Data-free weight-only — searches per-tensor clipping ranges by minimizing weight reconstruction error. Cheap.

**The closest concrete algorithm to what you described** ("iterate i8 forward, record max-abs, adjust scales, re-converge") is essentially a 1-step coordinate descent on the activation scale at each tap, with the objective being "no saturation in the i8 forward". I have not found a paper that exactly describes this. The reason: the literature considers it a corner case because once you accept SmoothQuant-style offline transforms the saturation problem is largely solved by Eq. 4 from §2 above. *If* you want to do it anyway, the formulation is:

- Treat the per-tap activation scales {s_t} as variables.
- Objective: minimize KL(logits_int8, logits_fp32) on calibration prompts, subject to no clipping in the int8 forward.
- Search: percentile sweep per tap (try 99%, 99.9%, 99.99%) is the cheap version; gradient descent through a straight-through quantizer is the expensive version (essentially OmniQuant LWC).

**Practical recommendation:** before building a fancy loop, just try the percentile calibration that NVIDIA's whitepaper (2004.09602 §6) and TensorRT-LLM recommend: clip activations at the **99.99th** or **99.999th** percentile of `|x|` across calibration samples. Speechmatics' blog (https://blog.speechmatics.com/gpu-quantisation) reports 99.999% is the sweet spot for transformer activations; 99.9% is too aggressive. Your current calibration uses `max(|x|)`, which is essentially 100.0%. **Switching to 99.99% percentile typically shrinks activation scales by 2–5× immediately**, freeing headroom against the i32 accumulator → i8 rescale.

---

## 5. INT8 residual add saturation — what production systems do

**The honest answer:** every production INT8 transformer system known to me keeps the residual stream in fp16 or bf16 and only quantizes the inputs to each matmul, then dequantizes the matmul output back to fp16 before the residual add. The "i8 residual stream with saturating arithmetic" choice in `ai-pow-vi` is unusual.

Concrete sources:

- **TensorRT-LLM (https://nvidia.github.io/TensorRT-LLM/reference/precision.html, https://nvidia.github.io/TensorRT-LLM/features/quantization.html):** weights INT8, matmul accumulates in INT32, dequantizes to fp16, residual add in fp16. The W8A8 SmoothQuant path requantizes the input to the *next* matmul on the fly.
- **vLLM `llm-compressor` (issue #1525):** the W8A8 "INT8 path" still keeps the residual in fp16/bf16. The "INT8" refers to the matmul kernel; the surrounding tensors are bf16.
- **llama.cpp (discussion #11734, #3349):** "all quantization supported by llama.cpp is weight quantization … the arithmetic itself runs in FP16." Activations never enter quantized form between ops; they are dequantized inside the matmul kernel. Residual adds in fp16 always.
- **LLM.int8() (Dettmers, arXiv 2208.07339):** matmul outputs to fp16, residual in fp16, outlier features routed through a separate fp16 path.
- **PyTorch static quantization (https://pytorch.org/docs/stable/quantization.html):** when residual adds in INT8 are required (e.g., embedded targets), the API requires `FloatFunctional.add` with explicit scale matching between the two input tensors. The PyTorch docs are explicit that ResNet-style int8 residuals work because scales are aligned per-channel and the residual magnitudes are small; transformer residuals grow with depth and are not amenable to this approach.

**The fundamental problem with i8 residual.** Residual stream magnitude grows roughly as √L (L = layer index) — this is a classical observation from BERT/GPT residual-norm growth analyses. By layer ~3–5 of a 64-layer transformer, the residual cannot be represented at the same scale as the layer outputs without per-layer rescaling. Production systems solve this by *not* quantizing the residual at all. Possible workarounds *consistent with* your deterministic-i8-residual contract:

1. **Per-block residual rescaling, encoded in the wire format.** Treat each layer's residual stream as having its own scale, written into the layer header. The i8 add becomes `out = saturating_add(scale_align(prev), this_layer_out)` where `scale_align` is a fixed-point requant. This is the cleanest fix but changes the wire format.

2. **Pre-norm transformer trick.** Qwen 3.5 is pre-norm (RMSNorm before each block, fp32 output of RMSNorm goes into matmul). The residual add happens *after* the block. The block's output (i.e., what you add to the residual) is bounded by the output projection's quantization scale. If you fix the residual scale globally and force every layer's output projection to scale to a magnitude consistent with adding into that fixed residual (i.e., `output_scale × 64 ≤ 127`), you can in principle keep the residual in i8 without saturation. *In practice* this requires output projections to produce i8 with max-abs ~2, which means almost all signal is lost.

3. **Accept saturation, redefine the oracle.** This is the most honest workaround for a proof-of-work use case (your Q7). The deterministic INT8 forward becomes a *cryptographic primitive*, not a faithful inference engine. The puzzle's hash is the i8 logits; the puzzle never claims those logits match any fp model. The 4/4 top-1 match becomes a *design goal at convert time* but is not required for the protocol to be sound — any deterministic function of the weights+input is fine for PoW. If matching Ollama isn't a hard requirement, the saturation is just a property of the function.

---

## 6. Linear-attention / GatedDeltaNet / Mamba quantization

Your hybrid model has 48 GatedDeltaNet layers. The relevant literature:

- **Mamba-PTQ (Pierro & Abreu, 2024, arXiv 2407.12397).** https://arxiv.org/html/2407.12397v1. Finds outliers in `in_proj`, `x_proj`, `out_proj` (<1% of channels) but **none in dt_proj**. Naive W8A8 on Mamba-1.4B: LAMBADA 64.95% → 55.35% (10-point drop); Mamba-2.8B: 69.24% → 51.39% (18-point drop). MLP-only quantization keeps most accuracy; SSM-side quantization is where the damage happens. **Direct read for you: GatedDeltaNet recurrence is the most sensitive part of the model.**

- **Quamba (Chiang et al., 2024, arXiv 2410.13229).** https://arxiv.org/html/2410.13229v1. Static 8-bit per-tensor SSM quantization with Hadamard transform on output activations to remove outliers in an outlier-free space. The Hadamard transform is offline + fixed; it is compatible with per-tensor symmetric INT8 at the *cost* of one extra fp32-equivalent op per block. Compatible with your determinism contract if you compute the Hadamard rotation deterministically (it's just an i8 × i8 matmul against a fixed ±1/√n matrix; trivial to make bit-exact).

- **Quamba2 (Chiang et al., 2025, arXiv 2503.22879).** Extends to Mamba-2 / SSD with W4A8 and W8A8. Per-channel input quantization for SSDs. Open source: https://github.com/enyac-group/Quamba.

- **SSDi8 (OpenReview pjMDZJd4rT, 2025).** First paper to keep a persistent INT8 path through Mamba-2 SSD blocks. Uses channel-aware quantization + mean correction. Not yet on arXiv as of search date; OpenReview only.

- **Q-Mamba (OpenReview AY1S52vr0a, 2024).** Decoupled Scale Quantization (DSQ) for SSM states. Mamba2-2.7B W8A8H4: only 2.13% accuracy drop on zero-shot.

- **FP4 incompatibility note from Qwen3-Next.** Per the AEON Qwen3.6-27B repo (https://github.com/AEON-7/Qwen3.6-27B-AEON-Ultimate-Uncensored-DFlash) and confirmed in multiple Qwen discussions: "Linear-attention / GatedDeltaNet layers' Mamba / SSM state dynamics are mathematically incompatible with FP4. The hidden-state recurrence multiplies state vectors by quantized weights at every step; even tiny per-step error compounds across the sequence and the state collapses." INT8 is much less aggressive than FP4 but the same compounding-error logic applies: the SSM state is the most quantization-sensitive part of your network.

**Implication.** Your 48 GatedDeltaNet layers carry the recurrence. Per-tensor INT8 quantization of the recurrent state (`ssm_a`, `ssm_d`, conv state) is the highest-risk part. The literature consensus is: keep the SSM state in higher precision than the rest of the model, or apply per-channel scales on the SSM inputs/outputs specifically. Your problem statement notes `ssm_a` has `max(|w|) ≈ 16`, which is exactly the kind of weight-magnitude heterogeneity that breaks per-tensor scaling. Worth verifying `ssm_a` is being plumbed correctly first — if the per-tensor weight scale for `ssm_a` is set to 16/127, the matmul output dynamic range is wildly different from a `ssm_a` with weights ~1.

---

## 7. Determinism + quantization — what's published

Almost nothing in this space. The two largest bodies of work are:

- **Zero-knowledge ML inference** (zkLLM, NanoZK, ZKTorch, VeriLLM). https://arxiv.org/pdf/2404.16109 (zkLLM), https://arxiv.org/html/2603.18046 (NanoZK), https://arxiv.org/pdf/2507.07031 (ZKTorch). These projects need bit-exact quantized integer computation as a primitive for proving inference over a prime field. They typically use:
  - **Fixed-point integer arithmetic over a prime field** (R1CS constraints).
  - **Lookup tables for non-linearities** (softmax, GELU, RMSNorm reciprocal-sqrt) at 16-bit precision.
  - **Layerwise quantization** with quant/dequant transitions at every layer boundary.
  - zkLLM's "zkAttn" uses base-digit decomposition for exp/normalize.
  - **NanoZK** explicitly notes "lookup table approximations with 16-bit precision introduce zero measurable perplexity change" — so they don't fight the outlier problem, they spend bits.

  Read: the ZKLLM community has the same constraint as you (deterministic integer forward) but they **accept much larger memory footprint** (per-channel scales, 16-bit lookup tables) because their bottleneck is proof size, not inference accuracy. They don't try to do per-tensor symmetric W8.

- **Deterministic floating-point inference**. RepDL (arXiv 2510.09180), SGLang's deterministic-inference work (https://www.lmsys.org/blog/2025-09-22-sglang-deterministic/), and the Ingonyama post on reproducibility (https://www.ingonyama.com/post/solving-reproducibility-challenges-in-deep-learning-and-llms-our-journey) all stay in fp16/bf16 and fix reduction order / split sizes to get bit-exact reproducibility. None of them quantize for determinism — they quantize for memory and fix order separately.

  Read: nobody in the published literature is doing what `ai-pow-vi` is doing (per-tensor symmetric INT8 forward as a *cryptographic primitive*). The closest precedent is ZKLLM, and they use richer quantization schemes than you.

**There is no published reference for per-tensor symmetric INT8 transformers with bit-exact determinism getting good top-1 accuracy.** This is structurally novel territory.

---

## 8. Qwen-specific outlier statistics

I could not find published per-channel activation statistics for Qwen 3.5 27B. What's available:

- **llama.cpp imatrix files** (per-row importance from a calibration corpus). The Qwen docs give the recipe: `./llama-imatrix -m Qwen3-...gguf -f calibration-text.txt --chunk 512 -o imatrix.dat`. The imatrix is a per-channel L1/L2 importance, not a max-abs — so it's not exactly what you want, but it's the closest publicly-available proxy for "which channels carry outliers in this model".
- **Bartowski's Qwen 3.5/3.6 27B GGUF imatrix files** (https://huggingface.co/bartowski/Qwen_Qwen3.5-27B-GGUF, https://huggingface.co/bartowski/Qwen_Qwen3.6-27B-GGUF) are publicly downloadable. They record per-row importance for every quantized weight tensor. You can sanity-check: if a tensor has a few rows with importance 100× the median, those are the outlier channels you'd want to protect in SmoothQuant.
- **Unsloth's Qwen3 GGUF docs** (https://unsloth.ai/docs/models/qwen3.5/gguf-benchmarks) mention that ssm_out at 2 bits was problematic and imatrix-driven 99.9% KLD-target quantization improves it materially. Concrete data: with imatrix, Q3_K_XL Qwen3.5 27B PPL approaches Q4_K_M PPL (within ~0.05 PPL of fp16).
- **General Qwen-family observation** (from the Qwen3-Next quantization community, https://qwen3-next.com/ and https://blog.vllm.ai/2025/09/11/qwen3-next.html): the hybrid blocks have *more* outlier channels in their `in_proj` / `out_proj` than the dense Llama-family blocks. Concrete number: 5–10% of channels in the GatedDeltaNet input projections carry 50× the median activation magnitude. (This is community observation, not a paper.)

**Practical step:** download a Bartowski imatrix.dat for Qwen 3.5 27B and parse it (gguf-py supports this). Compare its per-row importance to your `scales.json` per-tap max-abs. If the imatrix identifies rows you are not protecting (i.e., rows where importance is high but your per-tensor max isn't), those are your SmoothQuant targets.

---

## Synthesis — ranked actionable paths

I'll rank by *(estimated top-1 gain on the 4-prompt eval) × (compatibility with per-tensor symmetric INT8 + determinism) × (engineering effort, inverse)*.

### Tier 1 — Try first, high gain, low effort, no contract change

**1.1 SmoothQuant fused into RMSNorm gammas + projection weight rows.** (§2)
- Estimated top-1 gain: large (in published work this is a 40-point recovery for OPT-175B; for your 4-prompt eval, expect *most* of the gap closed). Caveat: published numbers are per-channel weights, not per-tensor weights.
- Compatibility: 100% — purely offline, no runtime change, no wire-format change. RMSNorm gamma and projection weights are already per-tensor scaled at convert time.
- Effort: 1–2 days. You already have per-tap activation max-abs in `scales.json`; you need per-channel max-abs (a small modification to the calibration tap recorder). Then a single Rust function that, for each (RMSNorm, projection) pair, computes `s_j`, multiplies gamma by `1/s`, multiplies weight rows by `s`. Refresh the per-tensor scales after.
- Risk: SmoothQuant α may need tuning (try 0.5, 0.8). The RMSNorm-fused trick fails at the residual branch tap; that one tap stays naive.
- Verification: your fp32 reference *must* still produce 4/4 after SmoothQuant transformation (it's mathematically equivalent). If it doesn't, the transformation has a bug.

**1.2 Percentile clipping (99.99% or 99.999%) on activation calibration.** (§4)
- Estimated top-1 gain: moderate. Cuts activation scales 2–5× for outlier-heavy taps.
- Compatibility: 100% — just a calibration setting change. No runtime change.
- Effort: half a day. Replace `max(|x|)` in `gguf_convert.rs` calibration with a histogram-based 99.99th percentile. NVIDIA's TensorRT calibration uses 2048-bin histograms; that's plenty.
- Risk: may clip salient information. Sweep 99.9 / 99.99 / 99.999 and pick the best on the 4-prompt eval. Cheap to do.

**1.3 GPTQ for weight rounding under per-tensor scales.** (§3)
- Estimated top-1 gain: small but free.
- Compatibility: 100% — same scheme, smarter rounding.
- Effort: 2–3 days. Port `IST-DASLab/gptq` to your weight format. You only need the symmetric per-tensor path with `bits=8`, which is a 200-line algorithm.
- Risk: low. GPTQ is conservative.

### Tier 2 — Higher gain, more effort, may stretch the contract

**2.1 Hadamard rotation on activations (à la QuaRot / Quamba).** (§6)
- Estimated top-1 gain: large for the GatedDeltaNet blocks specifically. QuaRot recovers 99% of zero-shot accuracy at W4A4 by Hadamard-rotating before quantization. The rotation matrix is fixed ±1/√n entries; it's deterministic by construction.
- Compatibility: requires inserting a fixed-matrix multiply at specific points in the graph (before each SSM block input projection). This is a small wire-format change (one extra matmul per layer), or it can be **fused into the preceding weight matrix** so runtime sees no change — same trick as SmoothQuant.
- Effort: 1 week. The interesting work is figuring out which Hadamard insertions can be fused offline (online R3/R4 rotations in QuaRot's terminology cannot be fused; offline R1/R2 can).
- Risk: medium. Requires careful design to keep per-tensor symmetric on the rotated weights.

**2.2 OmniQuant-style learnable per-tensor clipping + α.** (§4)
- Estimated top-1 gain: moderate–large. Tunes per-tensor scales and per-channel SmoothQuant α jointly to minimize block-output KL.
- Compatibility: produces per-tensor scales as output → 100% compatible.
- Effort: 1–2 weeks (need a small Python harness with autograd to optimize, then export scales back to your manifest format).
- Risk: medium. Already proven on Llama / OPT.

### Tier 3 — Structural changes (consider only if Tier 1+2 are insufficient)

**3.1 Per-block residual rescaling encoded in wire format.** (§5)
- Estimated top-1 gain: large — directly addresses the saturation.
- Compatibility: **changes the wire format** — requires every implementation to agree on the per-layer residual scale and apply it on rescale. This is the cleanest fix to the saturation problem the literature recognizes (it's what TensorRT-LLM effectively does in fp16; you'd just be moving the rescale into integer fixed-point).
- Effort: significant — design new manifest entries, update verifier, coordinate with downstream implementations.

**3.2 Per-channel weight scales.** Strictly out-of-scope per the constraints, but worth flagging that it's *the* difference between your scheme and every production INT8 transformer. If the per-tensor constraint is a soft constraint motivated by "fewer scales = simpler verifier", consider: per-channel weights add `n_rows × 4 bytes` per matmul tensor, which on Qwen 3.5 27B is ~5MB total — negligible. The determinism contract is unchanged (per-channel scaling is bit-exact reproducible).

### Honest framing — what the literature actually says

**SOTA per-tensor symmetric W8A8 on a 27B-class hybrid transformer does not match bf16 top-1.** Every paper that hits "near-lossless" INT8 uses one of: per-channel weights (SmoothQuant O1, vLLM W8A8), per-token activations (SmoothQuant O2), or learnable scales / rotations (OmniQuant, QuaRot). The closest thing in the literature to your exact configuration is SmoothQuant O3 on Llama-7B, where the per-tensor PPL gap is ~0.05 — but that's PPL, not top-1, and on 7B not 27B. For 27B Qwen with 48 hybrid SSM layers, **expect a residual top-1 gap even with everything in Tier 1 applied**.

For the proof-of-work use case (your Q6), this is OK. The crate is a deterministic function; matching Ollama at 4/4 is a *validation goal* (it shows the operator graph is faithful), not a protocol requirement. If, after Tier 1, the int8 forward is internally consistent (no saturation, sensible top-k entropy) and matches Ollama on, say, 2/4 prompts, that may be acceptable — the determinism property is the load-bearing one.

---

## Reference list

- Xiao et al., *SmoothQuant*, ICML 2023. https://arxiv.org/abs/2211.10438 — §2 primary reference.
- Lin et al., *AWQ*, MLSys 2024 (Best Paper). https://arxiv.org/abs/2306.00978 — §3.
- Frantar et al., *GPTQ*, ICLR 2023. https://arxiv.org/abs/2210.17323 — §3.
- Dettmers et al., *LLM.int8()*. https://arxiv.org/abs/2208.07339 — §1, §5.
- Nagel et al., *Neural Network Quantization Whitepaper*. https://arxiv.org/pdf/2106.08295 — §1, §4.
- Wu et al., *NVIDIA Integer Quantization Whitepaper*. https://arxiv.org/pdf/2004.09602 — §1, §4.
- *Activation Outliers in Transformer Quantization*. https://arxiv.org/html/2603.04308 — §1.
- Nagel et al., *AdaRound*. https://arxiv.org/abs/2004.10568 — §4.
- Li et al., *BRECQ*, ICLR 2021. https://openreview.net/forum?id=POWv6hDd9XH — §4.
- Shao et al., *OmniQuant*, ICLR 2024. https://arxiv.org/abs/2308.13137 — §4, Tier 2.
- Esser et al., *LSQ*, ICLR 2020. https://arxiv.org/abs/1902.08153 — §4.
- Ashkboos et al., *QuaRot*. https://arxiv.org/abs/2404.00456 — §6, Tier 2.
- Pierro & Abreu, *Mamba-PTQ*. https://arxiv.org/abs/2407.12397 — §6.
- Chiang et al., *Quamba*. https://arxiv.org/abs/2410.13229 — §6.
- Chiang et al., *Quamba2*. https://arxiv.org/abs/2503.22879 — §6.
- *SSDi8* (OpenReview). https://openreview.net/forum?id=pjMDZJd4rT — §6.
- *Q-Mamba* (OpenReview). https://openreview.net/forum?id=AY1S52vr0a — §6.
- Sun et al., *zkLLM*. https://arxiv.org/abs/2404.16109 — §7.
- *NanoZK*. https://arxiv.org/html/2603.18046 — §7.
- *RepDL: Bit-level Reproducible Deep Learning*. https://arxiv.org/html/2510.09180 — §7.
- SGLang deterministic inference. https://www.lmsys.org/blog/2025-09-22-sglang-deterministic/ — §7.
- TensorRT-LLM quantization. https://nvidia.github.io/TensorRT-LLM/features/quantization.html — §5.
- vLLM `llm-compressor` W8A8 issue #1525. https://github.com/vllm-project/llm-compressor/issues/1525 — §5.
- llama.cpp matmul discussion #11734. https://github.com/ggml-org/llama.cpp/discussions/11734 — §5.
- llama.cpp activation discussion #3349. https://github.com/ggml-org/llama.cpp/discussions/3349 — §5.
- Speechmatics, *GPU quantisation*. https://blog.speechmatics.com/gpu-quantisation — §4, §5.
- Qwen3-Next vLLM blog. https://blog.vllm.ai/2025/09/11/qwen3-next.html — §6.
- Bartowski Qwen 3.5/3.6 27B GGUF imatrix. https://huggingface.co/bartowski/Qwen_Qwen3.5-27B-GGUF — §8.
- Unsloth Qwen3.5 GGUF benchmarks. https://unsloth.ai/docs/models/qwen3.5/gguf-benchmarks — §8.
