-- 0002_functions.sql: the three SQL functions from docs/DESIGN.md 5.9. Each is security definer,
-- revoked from public and granted to service_role only. The calling code has no fallback path:
-- if a function is missing, the request fails (503).

-- activate_device: one activation per (entitlement, device) under an advisory lock, so
-- concurrent activations cannot exceed the limit. Web rows evict the least recently used
-- browser instead of refusing (DESIGN.md 5.1 item 7).
create or replace function public.activate_device(
  p_ent text,
  p_kind text,
  p_hash text,
  p_name text,
  p_platform text,
  p_version text
) returns jsonb
language plpgsql
security definer
set search_path = public
as $$
declare
  v_max int;
  v_count int;
  v_row devices%rowtype;
  v_devices jsonb;
begin
  perform pg_advisory_xact_lock(hashtextextended(p_ent, 0));

  select case when p_kind = 'web' then max_web else max_devices end
    into v_max
    from entitlements where id = p_ent;
  if v_max is null then
    raise exception 'entitlement % not found', p_ent;
  end if;

  select * into v_row
    from devices
   where entitlement_id = p_ent and device_hash = p_hash and deactivated_at is null;
  if found then
    update devices
       set last_seen_at = now(),
           app_version = coalesce(p_version, app_version),
           device_name = coalesce(p_name, device_name)
     where id = v_row.id
     returning * into v_row;
    select count(*) into v_count from devices where entitlement_id = p_ent and kind = p_kind and deactivated_at is null;
    return jsonb_build_object('ok', true, 'existing', true, 'used', v_count, 'max', v_max, 'device', to_jsonb(v_row));
  end if;

  select count(*) into v_count from devices where entitlement_id = p_ent and kind = p_kind and deactivated_at is null;

  if v_count >= v_max then
    if p_kind = 'web' then
      update devices
         set deactivated_at = now(), deactivated_by = 'evicted'
       where id = (
         select id from devices
          where entitlement_id = p_ent and kind = 'web' and deactivated_at is null
          order by last_seen_at asc
          limit 1
       );
    else
      select coalesce(jsonb_agg(jsonb_build_object('name', device_name, 'platform', platform, 'last_seen_at', last_seen_at) order by last_seen_at desc), '[]'::jsonb)
        into v_devices
        from devices
       where entitlement_id = p_ent and kind = 'desktop' and deactivated_at is null;
      return jsonb_build_object('ok', false, 'reason', 'limit', 'devices', v_devices);
    end if;
  end if;

  insert into devices (entitlement_id, kind, device_hash, device_name, platform, app_version)
  values (p_ent, p_kind, p_hash, p_name, p_platform, p_version)
  returning * into v_row;

  select count(*) into v_count from devices where entitlement_id = p_ent and kind = p_kind and deactivated_at is null;
  return jsonb_build_object('ok', true, 'existing', false, 'used', v_count, 'max', v_max, 'device', to_jsonb(v_row));
end;
$$;
revoke all on function public.activate_device(text, text, text, text, text, text) from public;
grant execute on function public.activate_device(text, text, text, text, text, text) to service_role;

-- consume_otp_attempt: ConvertSave's function adapted to code_hmac and used_at. The increment and
-- the cap check are one UPDATE, so parallel guesses cannot exceed p_max. Zero rows means missing,
-- used, or over the cap; the caller answers "invalid" for all three.
create or replace function public.consume_otp_attempt(
  p_email text,
  p_max int
) returns table(code_hmac bytea, expires_at timestamptz)
language plpgsql
security definer
set search_path = public
as $$
begin
  return query
  update otp_codes o
     set attempts = o.attempts + 1
   where o.email = p_email
     and o.used_at is null
     and o.attempts < p_max
  returning o.code_hmac, o.expires_at;
end;
$$;
revoke all on function public.consume_otp_attempt(text, int) from public;
grant execute on function public.consume_otp_attempt(text, int) to service_role;

-- rate_limit_hit: ConvertSave's function unchanged. Fixed window; returns true when the request
-- is allowed. The caller fails closed on any error.
create or replace function public.rate_limit_hit(
  p_key text,
  p_window_seconds int,
  p_max int
) returns boolean
language plpgsql
security definer
set search_path = public
as $$
declare
  v_bucket timestamptz;
  v_count int;
begin
  v_bucket := to_timestamp(floor(extract(epoch from now()) / p_window_seconds) * p_window_seconds);

  insert into rate_limits(key, window_start, count)
    values (p_key, v_bucket, 1)
  on conflict (key, window_start)
    do update set count = rate_limits.count + 1
  returning count into v_count;

  delete from rate_limits
    where key = p_key and window_start < v_bucket - make_interval(secs => p_window_seconds);

  return v_count <= p_max;
end;
$$;
revoke all on function public.rate_limit_hit(text, int, int) from public;
grant execute on function public.rate_limit_hit(text, int, int) to service_role;
