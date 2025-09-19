import { useEffect, useState } from 'react'

type SettingsMap = Record<string, string>

export default function Admin() {
  const [settings, setSettings] = useState<SettingsMap>({})
  const [loading, setLoading] = useState(false)
  const [message, setMessage] = useState('')

  async function loadSettings() {
    try {
      const res = await fetch('/api/v1/admin/settings')
      const data = await res.json()
      setSettings(data.settings || {})
    } catch (e) {
      setMessage('加载设置失败')
    }
  }

  useEffect(() => { loadSettings() }, [])

  function update(k: string, v: string) {
    setSettings(s => ({ ...s, [k]: v }))
  }

  async function save() {
    setLoading(true)
    setMessage('')
    try {
      const res = await fetch('/api/v1/admin/settings', {
        method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(settings)
      })
      if (!res.ok) throw new Error(await res.text())
      setMessage('保存成功（已即时生效）')
    } catch (e: any) {
      setMessage('保存失败：' + e.message)
    } finally { setLoading(false) }
  }

  async function upload(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setLoading(true)
    setMessage('')
    const fd = new FormData(e.currentTarget)
    try {
      const res = await fetch('/api/v1/admin/upload', { method: 'POST', body: fd })
      const data = await res.json()
      if (!res.ok) throw new Error(JSON.stringify(data))
      setMessage('上传/索引成功：' + JSON.stringify(data))
    } catch (err: any) {
      setMessage('上传失败：' + err.message)
    } finally {
      setLoading(false)
      e.currentTarget.reset()
    }
  }

  return (
    <main style={{ maxWidth: 1024, margin: '24px auto', fontFamily: 'system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial' }}>
      <h3>后台管理</h3>

      <section style={{ border: '1px solid #eee', padding: 16, borderRadius: 8 }}>
        <h4>全局设置</h4>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <div style={{ border: '1px solid #f0f0f0', borderRadius: 8, padding: 12 }}>
            <h5>模型设置</h5>
            <div style={{ display: 'grid', gridTemplateColumns: '200px 1fr', gap: 8, alignItems: 'center' }}>
              <label>OPENAI_CHAT_MODEL</label>
              <input required value={settings.OPENAI_CHAT_MODEL || ''} onChange={e=>update('OPENAI_CHAT_MODEL', e.target.value)} placeholder='gpt-4o' />

              <label>OPENAI_EMBED_MODEL</label>
              <input required value={settings.OPENAI_EMBED_MODEL || ''} onChange={e=>update('OPENAI_EMBED_MODEL', e.target.value)} placeholder='text-embedding-3-small' />
            </div>
          </div>
          <div style={{ border: '1px solid #f0f0f0', borderRadius: 8, padding: 12 }}>
            <h5>重排设置</h5>
            <div style={{ display: 'grid', gridTemplateColumns: '200px 1fr', gap: 8, alignItems: 'center' }}>
              <label>COHERE_API_KEY</label>
              <input type='password' value={settings.COHERE_API_KEY || ''} onChange={e=>update('COHERE_API_KEY', e.target.value)} placeholder='可选：使用 Cohere 重排' />

              <label>COHERE_RERANK_MODEL</label>
              <input value={settings.COHERE_RERANK_MODEL || 'rerank-multilingual-v3.0'} onChange={e=>update('COHERE_RERANK_MODEL', e.target.value)} />

              <label>RERANK_URL</label>
              <input value={settings.RERANK_URL || ''} onChange={e=>update('RERANK_URL', e.target.value)} placeholder='可选：自托管重排服务地址' />

              <label>RERANK_TOKEN</label>
              <input type='password' value={settings.RERANK_TOKEN || ''} onChange={e=>update('RERANK_TOKEN', e.target.value)} placeholder='可选：重排服务鉴权' />
            </div>
          </div>
        </div>
        <div style={{ marginTop: 12 }}>
          <button disabled={loading} onClick={save}>保存设置</button>
          <button disabled={loading} onClick={loadSettings} style={{ marginLeft: 8 }}>刷新</button>
        </div>
      </section>

      <section style={{ border: '1px solid #eee', padding: 16, borderRadius: 8, marginTop: 16 }}>
        <h4>文档上传与索引（支持 pdf 或 text）</h4>
        <form onSubmit={upload}>
          <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', gap: 8, alignItems: 'center' }}>
            <label>document_id</label>
            <input name='document_id' required placeholder='doc-id' />

            <label>chunk_size</label>
            <input name='chunk_size' type='number' defaultValue={1800} />

            <label>overlap</label>
            <input name='overlap' type='number' defaultValue={0} />

            <label>file</label>
            <input name='file' type='file' required />
          </div>
          <div style={{ marginTop: 12 }}>
            <button disabled={loading} type='submit'>上传并索引</button>
          </div>
        </form>
      </section>

      <section style={{ border: '1px solid #eee', padding: 16, borderRadius: 8, marginTop: 16 }}>
        <h4>URL 导入</h4>
        <UrlImport setMessage={setMessage} />
      </section>

      <section style={{ border: '1px solid #eee', padding: 16, borderRadius: 8, marginTop: 16 }}>
        <h4>PDF Glob 导入</h4>
        <PdfGlobImport setMessage={setMessage} />
      </section>

      {message && (
        <div style={{ marginTop: 16, padding: 12, border: '1px solid #ddd', borderRadius: 8, whiteSpace: 'pre-wrap' }}>{message}</div>
      )}
    </main>
  )
}

function UrlImport({ setMessage }: { setMessage: (s: string)=>void }) {
  const [url, setUrl] = useState('')
  const [doc, setDoc] = useState('')
  const [loading, setLoading] = useState(false)
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault(); setLoading(true); setMessage('')
    if (!url || !doc) { setMessage('URL 与 document_id 必填'); setLoading(false); return }
    try {
      const res = await fetch('/api/v1/documents/url', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ url, document_id: doc }) })
      const data = await res.json();
      if (!res.ok) throw new Error(JSON.stringify(data))
      setMessage('URL 导入成功：' + JSON.stringify(data))
      setUrl(''); setDoc('')
    } catch (e: any) { setMessage('URL 导入失败：' + e.message) } finally { setLoading(false) }
  }
  return (
    <form onSubmit={submit}>
      <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', gap: 8, alignItems: 'center' }}>
        <label>url</label>
        <input value={url} onChange={e=>setUrl(e.target.value)} placeholder='https://example.com' required />
        <label>document_id</label>
        <input value={doc} onChange={e=>setDoc(e.target.value)} placeholder='url-1' required />
      </div>
      <div style={{ marginTop: 12 }}>
        <button disabled={loading} type='submit'>导入 URL</button>
      </div>
    </form>
  )
}

function PdfGlobImport({ setMessage }: { setMessage: (s: string)=>void }) {
  const [glob, setGlob] = useState('')
  const [prefix, setPrefix] = useState('pdf-')
  const [loading, setLoading] = useState(false)
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault(); setLoading(true); setMessage('')
    if (!glob) { setMessage('glob 必填'); setLoading(false); return }
    try {
      const res = await fetch('/api/v1/documents/pdf_glob', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ glob, prefix }) })
      const data = await res.json();
      if (!res.ok) throw new Error(JSON.stringify(data))
      setMessage('PDF Glob 导入成功：' + JSON.stringify(data))
      setGlob('');
    } catch (e: any) { setMessage('PDF Glob 导入失败：' + e.message) } finally { setLoading(false) }
  }
  return (
    <form onSubmit={submit}>
      <div style={{ display: 'grid', gridTemplateColumns: '240px 1fr', gap: 8, alignItems: 'center' }}>
        <label>glob</label>
        <input value={glob} onChange={e=>setGlob(e.target.value)} placeholder='/path/to/*.pdf' required />
        <label>prefix</label>
        <input value={prefix} onChange={e=>setPrefix(e.target.value)} />
      </div>
      <div style={{ marginTop: 12 }}>
        <button disabled={loading} type='submit'>导入 PDF Glob</button>
      </div>
    </form>
  )
}
