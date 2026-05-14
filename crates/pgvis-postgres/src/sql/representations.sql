-- Data representations (domain type casts) introspection query.
-- Ported from PostgREST SchemaCache.hs `dataRepresentations`.
--
-- Parameters: none
--
-- Returns implicit casts involving domain types to/from json or text.
-- These enable transparent serialisation of domain types.
SELECT
  c.castsource::regtype::text AS source_type,
  c.casttarget::regtype::text AS target_type,
  c.castfunc::regproc::text AS function_name
FROM
  pg_catalog.pg_cast c
JOIN pg_catalog.pg_type src_t
  ON c.castsource::oid = src_t.oid
JOIN pg_catalog.pg_type dst_t
  ON c.casttarget::oid = dst_t.oid
WHERE
  c.castcontext = 'i'
  AND c.castmethod = 'f'
  AND has_function_privilege(c.castfunc, 'execute')
  AND ((src_t.typtype = 'd' AND c.casttarget IN ('json'::regtype::oid, 'text'::regtype::oid))
   OR (dst_t.typtype = 'd' AND c.castsource IN ('json'::regtype::oid, 'text'::regtype::oid)))
