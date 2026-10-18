# psycopg 3 fixture: COPY, server-side cursor, transaction + rollback, executemany, JSONB.
import os, sys
import psycopg
from psycopg.types.json import Jsonb

url = os.environ["DATABASE_URL"]
with psycopg.connect(url, autocommit=False) as conn:
    with conn.cursor() as cur:
        cur.execute("CREATE TABLE event (id serial PRIMARY KEY, kind text, payload jsonb)")
        cur.executemany("INSERT INTO event (kind, payload) VALUES (%s, %s)",
                        [("click", Jsonb({"n": i})) for i in range(1000)])
        conn.commit()
        # transaction + rollback
        cur.execute("INSERT INTO event (kind) VALUES ('temp')")
        conn.rollback()
        # COPY in
        with cur.copy("COPY event (kind, payload) FROM STDIN") as cp:
            for i in range(500):
                cp.write_row(["copy", '{"c": %d}' % i])
        conn.commit()
        total = cur.execute("SELECT count(*) FROM event").fetchone()[0]
        # server-side (named) cursor streaming
        seen = 0
        with conn.cursor(name="srv") as sc:
            sc.execute("SELECT id FROM event ORDER BY id")
            for _ in sc:
                seen += 1
        jb = cur.execute("SELECT payload->>'n' FROM event WHERE kind='click' AND payload->>'n'='7'").fetchone()[0]
        code = None
        try:
            cur.execute("CREATE TABLE event (x int)")
        except psycopg.errors.DuplicateTable as e:
            code = e.sqlstate
            conn.rollback()
        assert total == 1500, "expected 1500 rows, got %d" % total
        assert seen == 1500, "server-side cursor saw %d" % seen
        assert jb == "7", "jsonb wrong: %r" % jb
        assert code == "42P07", "expected 42P07, got %r" % code
print("APP python: PASS (rows=%d, server-side cursor ok, COPY ok, rollback ok, 42P07 ok)" % total)
