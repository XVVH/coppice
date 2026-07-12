# W-9 census — zero-authorship defaults over the tool universe

Universe: **9807 tools** across **1921 servers**.

| source | tools | servers | desc % | params % | annotations % | categories % | read/write/delete (heuristic) |
|---|---|---|---|---|---|---|---|
| mcp-flow | 7640 | 1563 | 99% | 100% | 0% | 100% | 2909/4483/248 |
| toucan | 2167 | 358 | 97% | 100% | 0% | 100% | 1013/1122/32 |

## Zero-authorship derivability (§0/§4, per registration field)

| field | derivable | % | note |
|---|---|---|---|
| side_effect | 0 | 0.0% | nothing declares locality; §0 floors every tool to external |
| egress | 0 | 0.0% | undeclared egress = egress on external open surfaces (§0) |
| reversibility | 0 | 0.0% | only MCP readOnlyHint/destructiveHint qualify as declared signal, and MCP itself marks them untrusted hints |
| action_class | 0 | 0.0% | same annotation dependence as reversibility |
| domain | 9807 | 100.0% | server-level categories/tags only; taxonomy governance stays open |
| store | 0 | 0.0% | SI-16 store binding has no foreign analogue; harness uses a representational ext:<server> |
| path_args | 0 | 0.0% | no schema names its path-carrying arguments; paths.write can only fail closed on write-shaped foreign calls |

**Conservative floor:** 100.0% of the universe is irreversible-egress under the free defaults.

## Encodability gaps (SI candidates, bounded samples)

```json
{
  "tool_server_ambiguous": {
    "count": 12,
    "samples": [
      "2b33f477-0368-512c-b5f8-58e699f07e55: 虚拟币价格查询服务-get_coin_price",
      "3d024251-1d9b-5ee5-8c90-27da920574a1: 虚拟币价格查询服务-get_coin_price",
      "450e900f-0d1a-5c0e-ae8c-4a2c053fcbdb: 虚拟币价格查询服务-get_coin_price"
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
