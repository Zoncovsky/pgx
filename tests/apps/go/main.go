// jackc/pgx v5 fixture: connect, batch, transaction + rollback, CopyFrom, prepared, JSONB.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/jackc/pgx/v5"
)

func main() {
	if err := run(); err != nil {
		fmt.Printf("APP go: FAIL (%v)\n", err)
		os.Exit(1)
	}
}

func run() error {
	ctx := context.Background()
	conn, err := pgx.Connect(ctx, os.Getenv("DATABASE_URL"))
	if err != nil {
		return err
	}
	defer conn.Close(ctx)
	if _, err = conn.Exec(ctx, `CREATE TABLE acct (id int PRIMARY KEY, name text, bal numeric, meta jsonb)`); err != nil {
		return err
	}
	// CopyFrom 2000 rows
	rows := make([][]any, 2000)
	for i := range rows {
		rows[i] = []any{i, fmt.Sprintf("a%d", i), (i + 1) * 10, map[string]int{"i": i}}
	}
	n, err := conn.CopyFrom(ctx, pgx.Identifier{"acct"}, []string{"id", "name", "bal", "meta"}, pgx.CopyFromRows(rows))
	if err != nil || n != 2000 {
		return fmt.Errorf("CopyFrom n=%d err=%v", n, err)
	}
	// transaction + rollback
	tx, err := conn.Begin(ctx)
	if err != nil {
		return err
	}
	if _, err = tx.Exec(ctx, `UPDATE acct SET bal = bal + 1000 WHERE id = 1`); err != nil {
		return err
	}
	if err = tx.Rollback(ctx); err != nil {
		return err
	}
	var bal int
	if err = conn.QueryRow(ctx, `SELECT bal FROM acct WHERE id = 1`).Scan(&bal); err != nil {
		return err
	}
	if bal != 20 {
		return fmt.Errorf("rollback failed, bal=%d (want 20)", bal)
	}
	// batch
	b := &pgx.Batch{}
	b.Queue(`UPDATE acct SET bal = bal * 2 WHERE id < $1`, 100)
	b.Queue(`SELECT count(*) FROM acct WHERE bal = 0`)
	br := conn.SendBatch(ctx, b)
	if _, err = br.Exec(); err != nil {
		return err
	}
	var zero int
	if err = br.QueryRow().Scan(&zero); err != nil {
		return err
	}
	if err = br.Close(); err != nil {
		return err
	}
	// prepared + jsonb + unique violation
	var total, meta int
	if err = conn.QueryRow(ctx, `SELECT count(*) FROM acct`).Scan(&total); err != nil {
		return err
	}
	if err = conn.QueryRow(ctx, `SELECT (meta->>'i')::int FROM acct WHERE id = 7`).Scan(&meta); err != nil {
		return err
	}
	_, uerr := conn.Exec(ctx, `INSERT INTO acct (id) VALUES (1)`)
	if uerr == nil {
		return fmt.Errorf("expected unique violation")
	}
	if total != 2000 || meta != 7 || zero != 0 {
		return fmt.Errorf("total=%d meta=%d zero=%d", total, meta, zero)
	}
	fmt.Printf("APP go: PASS (rows=%d, CopyFrom ok, rollback ok, batch ok, jsonb ok, unique rejected)\n", total)
	return nil
}
