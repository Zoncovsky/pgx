// node-postgres fixture: pool, parameterized inserts, transaction + rollback,
// prepared statement, JSONB, error SQLSTATE.
const { Client } = require('pg');
(async () => {
  const c = new Client({ connectionString: process.env.DATABASE_URL });
  await c.connect();
  try {
    await c.query('CREATE TABLE item (id serial PRIMARY KEY, sku text UNIQUE, qty int, meta jsonb)');
    for (let i = 0; i < 500; i++)
      await c.query('INSERT INTO item (sku, qty, meta) VALUES ($1, $2, $3)', ['sku' + i, i, { i }]);
    // transaction + rollback
    await c.query('BEGIN');
    await c.query("INSERT INTO item (sku, qty) VALUES ('temp', 1)");
    await c.query('ROLLBACK');
    // unique violation -> 23505
    let code = null;
    try { await c.query("INSERT INTO item (sku, qty) VALUES ('sku1', 1)"); }
    catch (e) { code = e.code; }
    if (code !== '23505') throw new Error('expected 23505, got ' + code);
    // prepared / named statement
    const r = await c.query({ name: 'byqty', text: 'SELECT count(*) FROM item WHERE qty < $1', values: [100] });
    const n = parseInt(r.rows[0].count, 10);
    const total = parseInt((await c.query('SELECT count(*) FROM item')).rows[0].count, 10);
    const age = (await c.query("SELECT meta->>'i' AS i FROM item WHERE sku = 'sku7'")).rows[0].i;
    if (total !== 500) throw new Error('expected 500 rows, got ' + total);
    if (n !== 100) throw new Error('expected 100 under qty 100, got ' + n);
    if (age !== '7') throw new Error('jsonb meta wrong: ' + age);
    console.log(`APP node: PASS (rows=${total}, rollback ok, 23505 ok, prepared=${n}, jsonb ok)`);
  } finally { await c.end(); }
})().catch((e) => { console.log('APP node: FAIL (' + e.message + ')'); process.exit(1); });
