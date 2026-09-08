-- Post-baseline migration ledger marker for releases after clean installation.
INSERT INTO schema_migrations (version) VALUES ('001_runtime_compat') ON CONFLICT (version) DO NOTHING;
