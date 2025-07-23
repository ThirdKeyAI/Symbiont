# Runtime Build Optimization

This document explains the optimizations implemented to reduce the GitHub workflow runtime compilation from over 1 hour to ~10-15 minutes.

## Key Optimizations

### 1. Docker Layer Caching
- **Registry Cache**: Added dual-layer caching with GitHub Actions cache and container registry
- **BuildKit Mount Cache**: Used mount cache for Cargo registry and build artifacts
- **Dependency Isolation**: Separate layer for dependency compilation vs source compilation

### 2. Rust Compilation Optimizations
- **Mold Linker**: Fast linker reduces linking time by 60-80%
- **Sparse Registry**: Faster dependency fetching with sparse protocol
- **Minimal Features**: Added `minimal` feature flag to reduce compiled dependencies
- **Optimized Profiles**: Per-dependency optimization levels in Cargo.toml

### 3. Workflow Optimizations
- **Path-Based Triggers**: Docker builds only trigger on dependency/build file changes
- **Fast CI**: Separate workflow for code-only changes using native Rust cache
- **Parallel Jobs**: Leverage GitHub Actions parallelism

### 4. Build Configuration

#### `.cargo/config.toml`
- Parallel build jobs (4)
- Fast linker configuration
- Optimized dependency compilation

#### Feature Flags
- `minimal`: Essential dependencies only (default for CI)
- `full`: All features for production builds
- Independent feature enabling for different use cases

### 5. Expected Performance

| Build Type | Before | After | Improvement |
|------------|--------|-------|-------------|
| Full Docker Build | 60+ min | 15-20 min | 70% faster |
| Incremental CI | 30+ min | 5-10 min | 80% faster |
| Code-only changes | 20+ min | 2-5 min | 85% faster |

### 6. Cache Strategy

#### GitHub Actions Cache
- Rust dependencies cached across builds
- Docker layer cache for base images
- Registry cache for compiled artifacts

#### Registry Cache
- Pre-built dependency layers
- Cross-platform compatibility
- Persistent across workflow runs

### 7. Monitoring

The optimizations include:
- Dependency change detection
- Build time metrics in workflow logs
- Cache hit rate reporting
- PR comments for dependency changes

### 8. Local Development

To use the same optimizations locally:

```bash
# Install mold linker (Ubuntu/Debian)
sudo apt-get install mold

# Use optimized build
cd runtime
cargo build --features minimal --bin symbiont-mcp
```

### 9. Troubleshooting

#### Cache Issues
- Clear cache: Manual workflow dispatch with force_rebuild=true
- Check cache usage in GitHub Actions logs

#### Build Failures
- Verify mold linker availability
- Check feature flag compatibility
- Review dependency version constraints

### 10. Future Optimizations

Potential additional improvements:
- Cross-compilation caching
- Incremental LTO builds
- Dependency pre-compilation images
- Build artifact reuse across workflows