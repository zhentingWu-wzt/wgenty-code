//! Plugin detail page template

pub fn render(plugin_id: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Plugin Details - Wgenty Code Marketplace</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            min-height: 100vh;
            color: #fff;
        }}

        .container {{
            max-width: 1000px;
            margin: 0 auto;
            padding: 2rem;
        }}

        .back-link {{
            color: #667eea;
            text-decoration: none;
            margin-bottom: 2rem;
            display: inline-block;
        }}

        .plugin-header {{
            display: flex;
            gap: 2rem;
            margin-bottom: 3rem;
            padding: 2rem;
            background: rgba(255,255,255,0.05);
            border-radius: 16px;
        }}

        .plugin-icon-large {{
            width: 120px;
            height: 120px;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            border-radius: 24px;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 3rem;
        }}

        .plugin-info {{
            flex: 1;
        }}

        .plugin-name {{
            font-size: 2rem;
            font-weight: 700;
            margin-bottom: 0.5rem;
        }}

        .plugin-meta {{
            color: #888;
            margin-bottom: 1rem;
        }}

        .plugin-stats {{
            display: flex;
            gap: 2rem;
            margin-bottom: 1.5rem;
        }}

        .stat {{
            text-align: center;
        }}

        .stat-value {{
            font-size: 1.5rem;
            font-weight: 600;
            color: #fff;
        }}

        .stat-label {{
            font-size: 0.875rem;
            color: #888;
        }}

        .install-btn-large {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #fff;
            border: none;
            padding: 1rem 2rem;
            border-radius: 8px;
            font-size: 1.1rem;
            cursor: pointer;
            transition: opacity 0.3s;
        }}

        .install-btn-large:hover {{
            opacity: 0.9;
        }}

        .section {{
            margin-bottom: 2rem;
            padding: 2rem;
            background: rgba(255,255,255,0.05);
            border-radius: 16px;
        }}

        .section-title {{
            font-size: 1.25rem;
            font-weight: 600;
            margin-bottom: 1rem;
        }}

        .section-content {{
            color: #a0a0a0;
            line-height: 1.8;
        }}

        .tags {{
            display: flex;
            flex-wrap: wrap;
            gap: 0.5rem;
        }}

        .tag {{
            background: rgba(102, 126, 234, 0.2);
            color: #667eea;
            padding: 0.25rem 0.75rem;
            border-radius: 20px;
            font-size: 0.875rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <a href="/" class="back-link">← Back to Marketplace</a>

        <div class="plugin-header">
            <div class="plugin-icon-large">🔌</div>
            <div class="plugin-info">
                <h1 class="plugin-name">{}</h1>
                <div class="plugin-meta">Loading plugin details...</div>
                <div class="plugin-stats">
                    <div class="stat">
                        <div class="stat-value" id="downloads">-</div>
                        <div class="stat-label">Downloads</div>
                    </div>
                    <div class="stat">
                        <div class="stat-value" id="rating">-</div>
                        <div class="stat-label">Rating</div>
                    </div>
                    <div class="stat">
                        <div class="stat-value" id="version">-</div>
                        <div class="stat-label">Version</div>
                    </div>
                </div>
                <button class="install-btn-large" onclick="installPlugin('{}')">Install Plugin</button>
            </div>
        </div>

        <div class="section">
            <h2 class="section-title">Description</h2>
            <div class="section-content" id="description">
                Loading...
            </div>
        </div>

        <div class="section">
            <h2 class="section-title">Tags</h2>
            <div class="tags" id="tags">
                Loading...
            </div>
        </div>
    </div>

    <script>
        const pluginId = '{}';

        async function loadPluginDetails() {{
            try {{
                const response = await fetch(`/api/plugins/${{pluginId}}`);
                const data = await response.json();

                if (data.success) {{
                    const plugin = data.data;
                    document.querySelector('.plugin-name').textContent = plugin.name;
                    document.querySelector('.plugin-meta').textContent = `by ${{plugin.author}} • ${{plugin.license}} license`;
                    document.getElementById('downloads').textContent = formatNumber(plugin.downloads);
                    document.getElementById('rating').textContent = '⭐ ' + plugin.rating;
                    document.getElementById('version').textContent = plugin.version;
                    document.getElementById('description').textContent = plugin.description;
                    document.getElementById('tags').innerHTML = plugin.tags.map(tag =>
                        `<span class="tag">${{tag}}</span>`
                    ).join('');
                }}
            }} catch (error) {{
                console.error('Failed to load plugin details:', error);
            }}
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

        loadPluginDetails();
    </script>
</body>
</html>"#,
        plugin_id, plugin_id, plugin_id
    )
}
