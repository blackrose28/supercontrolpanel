# Taste (Continuously Learned by [CommandCode][cmd])

[cmd]: https://commandcode.ai/

# tdd
- Use strict TDD (red-green-refactor) for ALL incremental changes, not just critical ones — write failing tests first before implementation. Confidence: 0.90

# architecture
- Keep App.tsx as a thin shell/placeholder; move all component logic, orchestration, and state management to separate page/hook/api layers. Confidence: 0.75

# ui
- Force all timestamp display in the UI to GMT+7 timezone. Confidence: 0.80

