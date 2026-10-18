# Using PGX from your application

`pgx url <db>` prints a standard `postgres://` URL. Point `DATABASE_URL` at it;
any PostgreSQL client or ORM works unchanged. Runnable fixtures for each stack
are in `tests/apps/` (all verified against the assembled distribution).

```sh
pgx create myapp
export DATABASE_URL="$(pgx url myapp)"
```

## Rails / ActiveRecord

`config/database.yml` reads `DATABASE_URL`, or set it in the environment. PGX
supports migrations up/down, `db:schema:load`, seeds, transactions with
savepoints, `RecordNotUnique` / `InvalidForeignKey` from the database, JSONB and
parallel tests. Example: `tests/apps/rails/app.rb`.

## Node.js (`pg`)

```js
const { Client } = require('pg');
const c = new Client({ connectionString: process.env.DATABASE_URL });
await c.connect();
```
Parameterized and named/prepared statements, transactions, JSONB, `COPY`.
Example: `tests/apps/node/app.js`.

## Python (`psycopg` 3)

```python
import psycopg
with psycopg.connect(os.environ["DATABASE_URL"]) as conn: ...
```
`executemany`, `COPY`, server-side cursors, JSONB. Example:
`tests/apps/python/app.py`.

## Go (`jackc/pgx` v5)

```go
conn, _ := pgx.Connect(ctx, os.Getenv("DATABASE_URL"))
```
`CopyFrom`, batches, transactions, JSONB. Example: `tests/apps/go/main.go`.

## psql

```sh
psql "$(pgx url myapp)"
psql "$(pgx url myapp --socket)"   # Unix socket, no password
```

## Agent workflow

An agent creates a disposable database per task, works against it, and drops it:

```sh
db="task_$(date +%s)"
pgx create "$db"
DATABASE_URL="$(pgx --json url "$db" | jq -r .url)"  # run migrations / tests
pgx drop "$db" --force
```

The `pgrun-branching` skill automates this lifecycle.
