# ADR 005: Optional local segmentation

Implement an optional BiRefNet lite ONNX pipeline with lazy ONNX Runtime loading, a cached worker session, and full-resolution PNG output. The real 12 MP end-to-end test passed in 19.1 seconds. DirectML failed with a GPU device-hung error on this host, so CPU is the verified fallback. The three-second target and Photos-quality parity have not passed.

Copilot+-only APIs exclude ordinary PCs. Restrictive or GPL model packages are excluded. The official BiRefNet model card declares MIT, but training-data provenance and exact optional runtime notices remain public-distribution review items. The core portable archive excludes the AI pack; local evaluation is available.

Evidence: [official model card](https://huggingface.co/ZhengPeng7/BiRefNet_lite), [ONNX Runtime](https://github.com/microsoft/onnxruntime), [DirectML requirements](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html).
