-- Add display_name to admin_users for profile display.
ALTER TABLE admin_users ADD COLUMN display_name TEXT;
UPDATE admin_users SET display_name = 'Saro' WHERE display_name IS NULL;
