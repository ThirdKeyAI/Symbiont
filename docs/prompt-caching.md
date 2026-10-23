# Optional prompt caching

The runtime SDK supports explicit `Off`, `Prefix` and `Rolling` cache modes on
`CloudInferenceProvider`. These settings are optional. Leaving them unset preserves
the previous request format, including the existing static prefix marker on direct
Anthropic requests. There is no new CLI flag or environment-variable default.

`Prefix` marks the stable system/tool prefix on direct Anthropic requests and the
system message on OpenRouter Anthropic models. `Rolling` adds the provider's
automatic conversation cache control, so the provider chooses the last eligible
block as the conversation grows. Both modes support five-minute and one-hour
TTLs. `Off` omits runtime cache markers; it cannot disable a provider's implicit
caching. Other provider/model combinations reject explicit Prefix or Rolling
selection before dispatch. Direct Bedrock Converse is not supported by this API.

```rust,no_run
use std::sync::Arc;
use symbi_runtime::reasoning::{
    providers::cloud::CloudInferenceProvider,
    prompt_cache::{CacheTtl, PromptCache, PromptCacheConfig},
};

let provider = CloudInferenceProvider::from_env()
    .expect("configure a supported provider")
    .with_prompt_cache(PromptCacheConfig {
        mode: PromptCache::Rolling,
        ttl: CacheTtl::FiveMinutes,
    })
    .with_cache_usage_observer(Arc::new(|event| {
        // Missing counts are None, not evidence of zero cache usage.
        let raw_tokens = event.reported.usage.total_tokens;
        let cache_reads = event.reported.cache.cache_read_tokens;
        let cache_writes = event.reported.cache.cache_creation_tokens;
        // Send these values to application-owned telemetry.
    }));
// Use this provider in the existing reasoning-loop builder or runner.
```

Direct SDK callers can override the default with
`InferenceOptions::with_prompt_cache` or `set_prompt_cache`. The typed setting is
stored in the reserved `extra.symbi_prompt_cache` field and included in the
inference contract. That internal field never reaches the provider. Combining
typed caching with raw `model`, `messages`, `system`, `tools` or `cache_control`
overrides is rejected because those overrides could contradict the selected mode.
Existing raw extras continue to work when no typed cache policy is selected.

The reasoning loop and audited response route resolve provider defaults before
hashing and reserving the request. Explicit provider defaults also change the
provider's configuration identity used by evaluated deployments. Custom provider
wrappers must forward `InferenceProvider::prepare_options` along with completion
and input reservation; custom audit integrations should call it before hashing.
The default method is a no-op for existing implementations.

## Usage and budgets

Cache counts are advisory subsets of raw input usage. On the Anthropic protocol,
raw input is the sum of uncached input, cache reads and cache creation. On chat
completions, `prompt_tokens` already includes the cached subsets. They must not be
added a second time. Raw input/output/total usage and all existing shared token
budget limits remain authoritative; cache hits do not release additional budget.

`ProviderUsage::from_anthropic` and `from_chat_completions` expose the same parser
to applications with their own HTTP transport. Absent or null cache fields remain
`None`. Malformed, overflowing or inconsistent counters are errors. A response
without complete raw usage retains the existing unknown-usage behavior.

The synchronous observer receives provider/model and usage only, without prompt,
credential or tool data. It is called after successful parsing and before the
loop's durable settlement. Keep it quick and non-panicking. It is not an audit
receipt, billing record or cost-budget enforcement hook. Applications sharing a
provider across runs must supply their own attribution; a per-run observer is the
simplest option. Existing public `Usage`, `InferenceOptions`, response and journal
shapes are unchanged.

## Adoption and verification

For applications such as symbi-codered, enable Rolling explicitly on the provider
used by the reasoning loop, retain stable system/tool content between turns, and
record optional read/write counts alongside raw usage. Applications using their
own transport can adopt the protocol parsers independently. Measure observed
cache hits and actual provider billing before selecting a TTL or estimating
savings. Model minimum prefix sizes, routing, expiration and provider pricing
affect results; short requests may never hit a cache.

The local HTTP workflow test covers both protocols through two loop turns, a tool
result, signed budget reservations and recovered accounting. A fixture reporting
90 cached tokens still charges all 105 raw tokens per turn. Invalid cache counts
prevent tool execution and retain the uncertain reservation. Unit tests cover
modes, TTLs, unknown counts, overflow, raw-override conflicts and unchanged legacy
request formats. These tests establish transport and accounting behavior, not
live cache-hit rates, monetary savings or provider retention guarantees.

For regulated deployments, confirm the selected provider's current retention,
workspace isolation and processing terms before opting in. No cross-tenant cache
store or cache eviction mechanism is introduced in the runtime.

Provider references: [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
and [OpenRouter prompt caching](https://openrouter.ai/docs/guides/best-practices/prompt-caching).
