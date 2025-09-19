/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  output: 'standalone',
  async rewrites() {
    // 开发环境将 /api 转发到后端 API（默认 8080）
    const apiBase = process.env.NEXT_PUBLIC_API_BASE || 'http://localhost:8080';
    return [
      { source: '/api/:path*', destination: `${apiBase}/api/:path*` },
    ]
  }
};
module.exports = nextConfig;
