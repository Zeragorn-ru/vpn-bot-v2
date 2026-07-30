-- Existing plaintext credentials cannot be migrated safely without the runtime
-- root key. Remove them and require an administrator to enter them again.
DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM information_schema.columns
    WHERE table_name = 'app_secrets'
      AND column_name = 'value'
      AND data_type = 'text'
  ) THEN
    TRUNCATE app_secrets;
    ALTER TABLE app_secrets ALTER COLUMN value TYPE bytea USING convert_to(value, 'UTF8');
  END IF;
END $$;
