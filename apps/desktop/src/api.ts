export interface HealthResponse {
  status: string;
  service: string;
  storage_mode: string;
  max_server_ttl_seconds: number;
}

const apiBaseUrl = import.meta.env.VITE_API_URL ?? "http://127.0.0.1:8787";

export async function fetchHealth(signal?: AbortSignal): Promise<HealthResponse> {
  const response = await fetch(`${apiBaseUrl}/healthz`, {
    headers: { Accept: "application/json" },
    signal,
  });

  if (!response.ok) {
    throw new Error(`Healthcheck failed with status ${response.status}`);
  }

  return response.json() as Promise<HealthResponse>;
}