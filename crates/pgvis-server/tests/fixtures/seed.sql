-- pgvis test seed data
-- Populates tables created by schema.sql

SET search_path = test, public;

-- Items
INSERT INTO test.items (id, name, price, category, description, in_stock) VALUES
    (1, 'Widget', 9.99, 'gadgets', 'A basic widget', true),
    (2, 'Gizmo', 24.50, 'gadgets', 'An advanced gizmo', true),
    (3, 'Doohickey', 3.75, 'tools', 'A simple doohickey', true),
    (4, 'Thingamajig', 99.99, 'tools', 'Premium thingamajig', false),
    (5, 'Whatchamacallit', 149.95, 'premium', 'Top-of-the-line widget', true),
    (6, 'Sprocket', 12.00, 'tools', NULL, true),
    (7, 'Flanged Bracket', 7.50, 'tools', 'Industrial bracket', true),
    (8, 'Quantum Flux Capacitor', 999.00, 'premium', 'For time travel enthusiasts', false),
    (9, 'Rubber Duck', 2.99, 'toys', 'Debugging companion', true),
    (10, 'Sonic Screwdriver', 42.00, 'gadgets', 'Multi-purpose tool', true);
SELECT setval('test.items_id_seq', 10);

-- Users
INSERT INTO test.users (id, name, email, role, age, data) VALUES
    (1, 'Alice', 'alice@example.com', 'admin', 30, '{"theme": "dark", "lang": "en"}'),
    (2, 'Bob', 'bob@example.com', 'user', 25, '{"theme": "light", "lang": "fr"}'),
    (3, 'Charlie', 'charlie@example.com', 'user', 35, '{"theme": "dark"}'),
    (4, 'Diana', 'diana@example.com', 'moderator', 28, '{}'),
    (5, 'Eve', NULL, 'user', NULL, NULL);
SELECT setval('test.users_id_seq', 5);

-- Orders
INSERT INTO test.orders (id, user_id, total, status, notes) VALUES
    (1, 1, 34.49, 'completed', 'First order'),
    (2, 1, 149.95, 'shipped', NULL),
    (3, 2, 3.75, 'pending', 'Waiting for stock'),
    (4, 3, 111.99, 'completed', NULL),
    (5, 2, 9.99, 'cancelled', 'Changed mind');
SELECT setval('test.orders_id_seq', 5);

-- Order items (M2M)
INSERT INTO test.order_items (order_id, item_id, quantity) VALUES
    (1, 1, 2),    -- Alice: 2x Widget
    (1, 2, 1),    -- Alice: 1x Gizmo
    (2, 5, 1),    -- Alice: 1x Whatchamacallit
    (3, 3, 1),    -- Bob: 1x Doohickey
    (4, 4, 1),    -- Charlie: 1x Thingamajig
    (4, 6, 1),    -- Charlie: 1x Sprocket
    (5, 1, 1);    -- Bob: 1x Widget (cancelled)

-- Projects
INSERT INTO test.projects (id, name, owner_id, budget, status, metadata) VALUES
    (1, 'Project Alpha', 1, 10000.00, 'active', '{"priority": "high"}'),
    (2, 'Project Beta', 2, 5000.00, 'active', '{"priority": "low"}'),
    (3, 'Project Gamma', 1, NULL, 'archived', '{}');
SELECT setval('test.projects_id_seq', 3);

-- Tasks
INSERT INTO test.tasks (id, project_id, title, done, priority, assigned_to) VALUES
    (1, 1, 'Design architecture', true, 1, 1),
    (2, 1, 'Implement core', false, 2, 2),
    (3, 1, 'Write tests', false, 3, NULL),
    (4, 2, 'Setup CI', true, 1, 3),
    (5, 2, 'Deploy staging', false, 2, 2),
    (6, 3, 'Final review', true, 1, 1);
SELECT setval('test.tasks_id_seq', 6);

-- JSON data
INSERT INTO test.json_data (id, data, metadata) VALUES
    (1, '{"name": "first", "tags": ["a", "b"], "nested": {"key": "val"}}', '{"version": 1}'),
    (2, '{"name": "second", "tags": ["c"], "nested": {"key": "other"}}', NULL),
    (3, '{"name": "third", "tags": [], "score": 100}', '{"version": 2}');
SELECT setval('test.json_data_id_seq', 3);

-- Compound PK
INSERT INTO test.compound_pk (k1, k2, value) VALUES
    (1, 'a', 'first'),
    (1, 'b', 'second'),
    (2, 'a', 'third');

-- No PK table
INSERT INTO test.no_pk (a, b) VALUES
    ('x', 1),
    ('y', 2),
    ('z', 3);

-- Unicode data
INSERT INTO test.unicode_data (id, label, description) VALUES
    (1, '日本語テスト', 'Japanese text'),
    (2, 'Ñoño', 'Spanish special chars'),
    (3, 'Ελληνικά', 'Greek text'),
    (4, '🎉 Party', 'Emoji test');
SELECT setval('test.unicode_data_id_seq', 4);

-- Nullable cols
INSERT INTO test.nullable_cols (id, required_col, optional_col, optional_int, optional_bool) VALUES
    (1, 'has_all', 'present', 42, true),
    (2, 'missing_some', NULL, NULL, false),
    (3, 'all_null', NULL, NULL, NULL);
SELECT setval('test.nullable_cols_id_seq', 3);

-- Menagerie (type testing)
INSERT INTO test.menagerie (id, col_int2, col_int4, col_int8, col_float4, col_float8,
    col_numeric, col_bool, col_text, col_varchar, col_char, col_uuid,
    col_date, col_time, col_timestamp, col_timestamptz, col_interval,
    col_json, col_jsonb, col_text_arr, col_int_arr, col_bytea) VALUES
    (1, 1, 100, 1000000000, 3.14, 2.718281828,
     12345.67890, true, 'hello world', 'varchar val', 'char      ',
     'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11',
     '2024-01-15', '14:30:00', '2024-01-15 14:30:00', '2024-01-15 14:30:00+00',
     '1 year 2 months 3 days',
     '{"key": "value"}', '{"key": "value"}',
     ARRAY['one', 'two', 'three'], ARRAY[1, 2, 3],
     E'\\x48656c6c6f');
SELECT setval('test.menagerie_id_seq', 1);
