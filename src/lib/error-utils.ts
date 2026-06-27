const MAX_ERROR_PARSE_LENGTH = 100_000;
const BASE_RETRY_DELAY_MS = 3000;
const MAX_RETRIES = 3;

export function extractErrorMessage(raw: string | null | undefined): string | null {
  if (!raw) return null;
  if (raw.length > MAX_ERROR_PARSE_LENGTH) return raw;
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed === "string") return parsed;
    if (typeof parsed?.error?.message === "string") return parsed.error.message;
    if (Array.isArray(parsed?.error)) {
      const first = parsed.error[0];
      if (typeof first?.message === "string") return first.message;
    }
    if (typeof parsed?.message === "string") return parsed.message;
    const nested = parsed?.error;
    if (nested && typeof nested === "object" && !Array.isArray(nested)) {
      if (typeof nested.message === "string") return nested.message;
    }
    return raw;
  } catch {
    return raw;
  }
}

export function isRetryableError(error: string | null | undefined): boolean {
  if (!error) return false;
  if (/\b(400|403|404|401)\b|permission_denied|capability_not_found|bad_request/i.test(error)) {
    return false;
  }
  return (
    /\btimeout\b/i.test(error) ||
    /\btimed.?out\b/i.test(error) ||
    /deadline\s+has\s+elapsed/i.test(error) ||
    /超时/.test(error) ||
    /频率|限流|流控|try.again|later/i.test(error) ||
    /"code":\s*"429/.test(error) ||
    /\b429\b/.test(error) ||
    /rate\s*limit/i.test(error) ||
    /^5\d{2}\b/.test(error) ||
    /5xx|server\s*error|service\s*unavailable/i.test(error)
  );
}

export function calculateBackoffMs(attempt: number): number {
  return BASE_RETRY_DELAY_MS * Math.pow(2, Math.max(0, attempt - 1));
}

export { MAX_RETRIES, BASE_RETRY_DELAY_MS };
