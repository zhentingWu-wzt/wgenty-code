//! Index page template

pub fn render() -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Wgenty Code Plugin Marketplace</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            min-height: 100vh;
            color: #fff;
        }}

        .container {{
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
        }}

        header {{
            text-align: center;
            padding: 3rem 0;
        }}

        h1 {{
            font-size: 3rem;
            margin-bottom: 1rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
            background-clip: text;
        }}

        .subtitle {{
            font-size: 1.2rem;
            color: #a0a0a0;
            margin-bottom: 2rem;
        }}

        .search-box {{
            max-width: 600px;
            margin: 0 auto 3rem;
            position: relative;
        }}

        .search-box input {{
            width: 100%;
            padding: 1rem 1.5rem;
            font-size: 1.1rem;
            border: 2px solid rgba(255,255,255,0.1);
            border-radius: 50px;
            background: rgba(255,255,255,0.05);
            color: #fff;
            outline: none;
            transition: all 0.3s;
        }}

        .search-box input:focus {{
            border-color: #667eea;
            background: rgba(255,255,255,0.1);
        }}

        .search-box input::placeholder {{
            color: #666;
        }}

        .categories {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
            margin-bottom: 3rem;
        }}

        .category {{
            background: rgba(255,255,255,0.05);
            padding: 1.5rem;
            border-radius: 12px;
            text-align: center;
            cursor: pointer;
            transition: all 0.3s;
            border: 1px solid rgba(255,255,255,0.1);
        }}

        .category:hover {{
            background: rgba(255,255,255,0.1);
            transform: translateY(-2px);
        }}

        .category-icon {{
            font-size: 2rem;
            margin-bottom: 0.5rem;
        }}

        .category-name {{
            font-weight: 600;
            margin-bottom: 0.25rem;
        }}

        .category-count {{
            font-size: 0.875rem;
            color: #888;
        }}

        .section {{
            margin-bottom: 3rem;
        }}

        .section-header {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 1.5rem;
        }}

        .section-title {{
            font-size: 1.5rem;
            font-weight: 600;
        }}

        .view-all {{
            color: #667eea;
            text-decoration: none;
            font-size: 0.875rem;
        }}

        .plugins-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
            gap: 1.5rem;
        }}

        .plugin-card {{
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            padding: 1.5rem;
            border: 1px solid rgba(255,255,255,0.1);
            transition: all 0.3s;
        }}

        .plugin-card:hover {{
            background: rgba(255,255,255,0.08);
            transform: translateY(-2px);
        }}

        .plugin-header {{
            display: flex;
            align-items: flex-start;
            gap: 1rem;
            margin-bottom: 1rem;
        }}

        .plugin-icon {{
            width: 48px;
            height: 48px;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            border-radius: 12px;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 1.5rem;
        }}

        .plugin-info {{
            flex: 1;
        }}

        .plugin-name {{
            font-weight: 600;
            font-size: 1.1rem;
            margin-bottom: 0.25rem;
        }}

        .plugin-author {{
            font-size: 0.875rem;
            color: #888;
        }}

        .plugin-description {{
            color: #a0a0a0;
            font-size: 0.9rem;
            line-height: 1.5;
            margin-bottom: 1rem;
        }}

        .plugin-footer {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            font-size: 0.875rem;
        }}

        .plugin-stats {{
            display: flex;
            gap: 1rem;
            color: #888;
        }}

        .plugin-rating {{
            color: #ffd700;
        }}

        .install-btn {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #fff;
            border: none;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            cursor: pointer;
            font-size: 0.875rem;
            transition: opacity 0.3s;
        }}

        .install-btn:hover {{
            opacity: 0.9;
        }}

        .badge {{
            display: inline-block;
            padding: 0.25rem 0.5rem;
            border-radius: 4px;
            font-size: 0.75rem;
            font-weight: 600;
            margin-left: 0.5rem;
        }}

        .badge-official {{
            background: #4CAF50;
            color: #fff;
        }}

        .badge-verified {{
            background: #2196F3;
            color: #fff;
        }}

        footer {{
            text-align: center;
            padding: 3rem 0;
            color: #666;
            border-top: 1px solid rgba(255,255,255,0.1);
            margin-top: 3rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>🔌 Wgenty Code Plugin Marketplace</h1>
            <p class="subtitle">Discover and install plugins to extend Wgenty Code's capabilities</p>

            <div class="search-box">
                <input type="text" placeholder="Search plugins..." id="searchInput">
            </div>
        </header>

        <div class="categories">
            <div class="category">
                <div class="category-icon">🛠️</div>
                <div class="category-name">Tools</div>
                <div class="category-count">45 plugins</div>
            </div>
            <div class="category">
                <div class="category-icon">🔌</div>
                <div class="category-name">Integrations</div>
                <div class="category-count">32 plugins</div>
            </div>
            <div class="category">
                <div class="category-icon">🎨</div>
                <div class="category-name">Themes</div>
                <div class="category-count">18 plugins</div>
            </div>
            <div class="category">
                <div class="category-icon">🌐</div>
                <div class="category-name">Languages</div>
                <div class="category-count">24 plugins</div>
            </div>
            <div class="category">
                <div class="category-icon">⚡</div>
                <div class="category-name">Productivity</div>
                <div class="category-count">28 plugins</div>
            </div>
            <div class="category">
                <div class="category-icon">💻</div>
                <div class="category-name">Development</div>
                <div class="category-count">9 plugins</div>
            </div>
        </div>

        <div class="section">
            <div class="section-header">
                <h2 class="section-title">🔥 Trending</h2>
                <a href="/search?sort=downloads" class="view-all">View all →</a>
            </div>
            <div class="plugins-grid" id="trendingPlugins">
                <!-- Plugins will be loaded here -->
            </div>
        </div>

        <div class="section">
            <div class="section-header">
                <h2 class="section-title">✨ New Arrivals</h2>
                <a href="/search?sort=newest" class="view-all">View all →</a>
            </div>
            <div class="plugins-grid" id="newPlugins">
                <!-- Plugins will be loaded here -->
            </div>
        </div>

        <div class="section">
            <div class="section-header">
                <h2 class="section-title">⭐ Official Plugins</h2>
                <a href="/search?filter=official" class="view-all">View all →</a>
            </div>
            <div class="plugins-grid" id="officialPlugins">
                <!-- Plugins will be loaded here -->
            </div>
        </div>

        <footer>
            <p>Wgenty Code Plugin Marketplace v{}</p>
            <p style="margin-top: 0.5rem; font-size: 0.875rem;">
                <a href="/api/health" style="color: #667eea;">API Health</a> •
                <a href="/api/docs" style="color: #667eea;">API Docs</a>
            </p>
        </footer>
    </div>

    <script>
        // Load featured plugins
        async function loadFeatured() {{
            try {{
                const response = await fetch('/api/featured');
                const data = await response.json();

                if (data.success) {{
                    renderPlugins('trendingPlugins', data.data.trending);
                    renderPlugins('newPlugins', data.data.newest);
                    renderPlugins('officialPlugins', data.data.official);
                }}
            }} catch (error) {{
                console.error('Failed to load plugins:', error);
            }}
        }}

        function renderPlugins(containerId, plugins) {{
            const container = document.getElementById(containerId);
            container.innerHTML = plugins.map(plugin => `
                <div class="plugin-card">
                    <div class="plugin-header">
                        <div class="plugin-icon">🔌</div>
                        <div class="plugin-info">
                            <div class="plugin-name">
                                ${{plugin.name}}
                                ${{plugin.is_official ? '<span class="badge badge-official">Official</span>' : ''}}
                                ${{plugin.is_verified ? '<span class="badge badge-verified">Verified</span>' : ''}}
                            </div>
                            <div class="plugin-author">by ${{plugin.author}}</div>
                        </div>
                    </div>
                    <div class="plugin-description">${{plugin.description}}</div>
                    <div class="plugin-footer">
                        <div class="plugin-stats">
                            <span>⬇️ ${{formatNumber(plugin.downloads)}}</span>
                            <span class="plugin-rating">⭐ ${{plugin.rating}}</span>
                        </div>
                        <button class="install-btn" onclick="installPlugin('${{plugin.id}}')">Install</button>
                    </div>
                </div>
            `).join('');
        }}

        function formatNumber(num) {{
            if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M';
            if (num >= 1000) return (num / 1000).toFixed(1) + 'K';
            return num.toString();
        }}

        async function installPlugin(pluginId) {{
            try {{
                const response = await fetch(`/api/plugins/${{pluginId}}/install`, {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ plugin_id: pluginId }})
                }});
                const data = await response.json();

                if (data.success) {{
                    alert('Plugin installed successfully!');
                }} else {{
                    alert('Failed to install plugin: ' + data.error);
                }}
            }} catch (error) {{
                alert('Failed to install plugin: ' + error.message);
            }}
        }}

        // Search functionality
        document.getElementById('searchInput').addEventListener('keypress', function(e) {{
            if (e.key === 'Enter') {{
                window.location.href = '/search?q=' + encodeURIComponent(this.value);
            }}
        }});

        // Load plugins on page load
        loadFeatured();
    </script>
</body>
</html>"#,
        env!("CARGO_PKG_VERSION")
    )
}
