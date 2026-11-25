class HttpAssertion extends Error {
  constructor(status) {
    super("Unexpected HTTP status");
    this.status = status;
  }
}

export function requireHttpOk(response) {
  if (!response.ok()) {
    const status = response.status();
    throw new HttpAssertion(Number.isInteger(status) && status >= 100 && status <= 599 ? status : null);
  }
}

export function browserFailure(error) {
  if (error instanceof HttpAssertion) {
    return { code: "HTTP_STATUS", expected: "2xx", actual: error.status };
  }
  return { code: error?.code === "ERR_ASSERTION" ? "BROWSER_ASSERTION" : "BROWSER_EXECUTION" };
}
