-- Schema: core metadata for KB Platform
-- Enable pgcrypto for gen_random_uuid()
create extension if not exists pgcrypto;

create table if not exists tenants (
  id uuid primary key default gen_random_uuid(),
  name text not null unique,
  created_at timestamptz not null default now()
);

create table if not exists sources (
  id uuid primary key default gen_random_uuid(),
  tenant_id uuid not null references tenants(id) on delete cascade,
  kind text not null, -- upload, web, git, s3, confluence, notion, etc
  config jsonb not null,
  created_at timestamptz not null default now()
);

create table if not exists documents (
  id uuid primary key default gen_random_uuid(),
  tenant_id uuid not null references tenants(id) on delete cascade,
  source_id uuid references sources(id),
  title text,
  uri text not null, -- object storage path
  version text not null,
  sha256 char(64) not null,
  mime_type text,
  tags text[] default '{}',
  visibility text not null default 'private', -- private/tenant/public
  created_at timestamptz not null default now()
);

create table if not exists chunks (
  id uuid primary key default gen_random_uuid(),
  document_id uuid not null references documents(id) on delete cascade,
  ord integer not null, -- chunk order
  page integer, -- optional page number
  start_offset integer,
  end_offset integer,
  text text not null,
  vector_id text, -- id in vector store
  metadata jsonb not null default '{}',
  created_at timestamptz not null default now()
);
create index if not exists idx_chunks_doc on chunks(document_id);

create table if not exists index_jobs (
  id uuid primary key default gen_random_uuid(),
  tenant_id uuid not null references tenants(id) on delete cascade,
  document_ids uuid[] not null,
  run_graph boolean not null default true,
  run_vector boolean not null default true,
  run_lexical boolean not null default true,
  status text not null default 'queued', -- queued/running/succeeded/failed
  attempts int not null default 0,
  error text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

-- Graph model (simplified)
create table if not exists graph_nodes (
  id uuid primary key default gen_random_uuid(),
  tenant_id uuid not null references tenants(id) on delete cascade,
  label text not null, -- entity type
  name text not null,
  confidence real,
  source_chunk_id uuid references chunks(id),
  created_at timestamptz not null default now()
);
create table if not exists graph_edges (
  id uuid primary key default gen_random_uuid(),
  tenant_id uuid not null references tenants(id) on delete cascade,
  src uuid not null references graph_nodes(id) on delete cascade,
  dst uuid not null references graph_nodes(id) on delete cascade,
  label text not null, -- relation type
  confidence real,
  source_chunk_id uuid references chunks(id),
  created_at timestamptz not null default now()
);
create index if not exists idx_edges_src on graph_edges(src);
create index if not exists idx_edges_dst on graph_edges(dst);

create table if not exists query_logs (
  id bigserial primary key,
  tenant_id uuid not null references tenants(id) on delete cascade,
  user_id text,
  mode text not null, -- rag/graph/hybrid/lexical
  query text not null,
  params jsonb not null,
  answer text,
  citations jsonb,
  latency_ms integer,
  created_at timestamptz not null default now()
);
