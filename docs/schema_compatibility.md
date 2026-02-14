# YAML Schema Compatibility Policy

## Version 1.0 (Current)

The following schemas are frozen as v1.0:
- `chip.yaml` (ChipDescriptor)
- `system.yaml` (SystemManifest)
- `peripheral.yaml` (PeripheralDescriptor)

### Stability Guarantees

1. **Backwards Compatibility**: All v1.0 schemas will be supported indefinitely
2. **Additive Changes Only**: New optional fields may be added to v1.0 schemas
3. **No Breaking Changes**: Required fields and their semantics will not change
4. **Deprecation Policy**: Deprecated fields will be supported for at least 2 major versions

### Schema Evolution

When breaking changes are required:
1. Increment schema version (e.g., `schema_version: "2.0"`)
2. Support both versions for at least 6 months
3. Provide migration tooling (`labwired asset migrate`)
4. Document migration path in release notes

### Schema Version Field

All YAML configuration files support an optional `schema_version` field:

```yaml
schema_version: "1.0"
name: "my-chip"
# ... rest of config
```

If omitted, the schema version defaults to `"1.0"` for backwards compatibility.

## Version History

### v1.0 (2026-02-14)
- Initial frozen schema version
- Includes: ChipDescriptor, SystemManifest, PeripheralDescriptor
- Default version for all existing configs without explicit `schema_version` field
