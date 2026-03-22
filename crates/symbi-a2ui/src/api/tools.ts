export interface ToolSummary {
  name: string;
  mode: string;
  binary: string;
  risk_tier: string;
  cedar_resource: string;
  description: string;
}

// Note: There's no dedicated tools API endpoint yet.
// For now, this returns mock data or reads from a static endpoint.
// When symbi tools list becomes an API, this will fetch from /api/v1/tools.
export async function listTools(): Promise<ToolSummary[]> {
  // Placeholder — will be connected when API endpoint exists
  return [];
}
