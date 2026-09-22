declare namespace Cloudflare {
  interface Env extends import("../src/env").Env {
    SCHEMA_SQL: string;
  }
}
