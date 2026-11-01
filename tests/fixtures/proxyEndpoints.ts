import type { ProxyEndpoint } from "@/types/proxy";

// Test inventory only. Production reads the backend registry via IPC.
const routes: [ProxyEndpoint["listener"], ProxyEndpoint["method"], string][] = [
  ["responses", "POST", "/v1/responses"],
  ["responses", "POST", "/v1/chat/completions"],
  ["responses", "POST", "/v1/systemone"],
  ["responses", "WS", "/v1/responses"],
  ["responses", "POST", "/v1/responses/compact"],
  ["responses", "POST", "/v1/alpha/search"],
  ["responses", "POST", "/v1/images/generations"],
  ["responses", "POST", "/v1/images/edits"],
  ["responses", "GET", "/v1/models"],
  ["messages", "POST", "/v1/messages"],
  ["messages", "POST", "/v1/messages/count_tokens"],
  ["messages", "GET", "/v1/models"],
];

export const proxyEndpoints: ProxyEndpoint[] = routes.map(
  ([listener, method, path], index) => ({
    id: `endpoint-${index}`,
    listener,
    method,
    path,
    description: `Description ${index}`,
    sampleInput: `Sample request ${index}`,
    sampleOutput: `Sample response ${index}`,
    implemented: index !== 3,
  }),
);
