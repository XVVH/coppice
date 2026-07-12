# W-9 census — zero-authorship defaults over the tool universe

Universe: **9795 tools**. Server counts are per-source only — mcp-flow server identity is best-effort name-prefix recovery (an upper bound, not distinct servers); see census.json `servers_note`.

| source | tools | servers (upper bound) | desc % | params % | annotations % | declared categories % | read/write/delete (heuristic) |
|---|---|---|---|---|---|---|---|
| mcp-flow | 7640 | 1563 | 99% | 100% | 0% | 0% | 1915/5538/187 |
| toucan | 2155 | 357 | 97% | 100% | 0% | 100% | 1003/1120/32 |

## Zero-authorship derivability (§0/§4, per registration field)

| field | derivable | % | note |
|---|---|---|---|
| side_effect | 0 | 0.0% | nothing declares locality; §0 floors every tool to external |
| egress | 0 | 0.0% | undeclared egress = egress on external open surfaces (§0) |
| reversibility | 0 | 0.0% | tools carrying readOnlyHint or destructiveHint (set membership, not a key sum); MCP itself marks these untrusted hints |
| action_class | 0 | 0.0% | same reversibility-shaped hints; idempotent/openWorld-only annotations classify nothing |
| domain | 2155 | 22.0% | corpus-DECLARED server categories only (Toucan crawler labels — third-party, not §4 self-declared registration metadata); harness provenance tags count as nothing; taxonomy governance stays open |
| store | 0 | 0.0% | SI-16 store binding has no foreign analogue; harness uses a representational ext:<server> |
| path_args | 0 | 0.0% | no schema names its path-carrying arguments; paths.write can only fail closed on write-shaped foreign calls |

**Conservative floor:** 100.0% of the universe is irreversible-egress under the free defaults.

## Encodability gaps (SI candidates, bounded samples)

```json
{
  "server_name_not_sluggable": {
    "count": 18,
    "samples": [
      "774640e2-405c-5993-b032-c24824824858: 虚拟币价格查询服务",
      "7e3aebdc-0b20-562e-a4ab-45273a1a958d: 虚拟币价格查询服务",
      "be50a227-5bca-5427-8999-8100a481b5f5: 虚拟币价格查询服务"
    ]
  },
  "unparseable_call_arguments": {
    "count": 66,
    "samples": [
      "cd6aabab-ace0-53cf-84db-0766ab781830: clear-thought-server-scientificmethod: \"{\\\"inquiryId\\\": \\\"flash-sale-500-errors\\\", \\\"stage\\\": \\\"question\\\", \\\"iteration\\\": 1, \\\"nextStageNeeded\\\": true, \\\"ques",
      "8694a637-351f-5f21-af4b-7a927ce4bd68: clear-thought-server-visualreasoning: \"{\\\"operation\\\": \\\"create\\\", \\\"diagramId\\\": \\\"system-architecture-analysis\\\", \\\"diagramType\\\": \\\"flowchart\\\", \\\"iteratio",
      "506130cb-7afd-5f42-8b99-d52369ab922f: think-tank-upsert_entities: \"{\\\"entities\\\": [{\\\"name\\\": \\\"Research Phase 1: Literature Collection\\\", \\\"entityType\\\": \\\"Task\\\", \\\"observations\\\": [\\\""
    ]
  }
}
```
