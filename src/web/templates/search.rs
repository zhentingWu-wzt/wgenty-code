//! Search page template

pub fn render() -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Search Plugins - Wgenty Code Marketplace</title>
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
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
        }}

        .search-header {{
            margin-bottom: 2rem;
        }}

        .back-link {{
            color: #667eea;
            text-decoration: none;
            margin-bottom: 1rem;
            display: inline-block;
        }}

        .search-box {{
            display: flex;
            gap: 1rem;
            margin-bottom: 1.5rem;
        }}

        .search-box input {{
            flex: 1;
            padding: 1rem 1.5rem;
            font-size: 1.1rem;
            border: 2px solid rgba(255,255,255,0.1);
            border-radius: 8px;
            background: rgba(255,255,255,0.05);
            color: #fff;
            outline: none;
        }}

        .search-box input:focus {{
            border-color: #667eea;
        }}

        .filters {{
            display: flex;
            gap: 1rem;
            flex-wrap: wrap;
            margin-bottom: 2rem;
        }}

        .filter select {{
            padding: 0.5rem 1rem;
            background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.1);
            border-radius: 6px;
            color: #fff;
            outline: none;
        }}

        .results-count {{
            color: #888;
            margin-bottom: 1.5rem;
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
        }}

        .plugin-author {{
            font-size: 0.875rem;
            color: #888;
        }}

        .plugin-description {{
            color: #a0a0a0;
            font-size: 0.9rem;
            margin-bottom: 1rem;
        }}

        .plugin-footer {{
            display: flex;
            justify-content: space-between;
            align-items: center;
        }}

        .plugin-stats {{
            display: flex;
            gap: 1rem;
            color: #888;
            font-size: 0.875rem;
        }}

        .pagination {{
            display: flex;
            justify-content: center;
            gap: 0.5rem;
            margin-top: 3rem;
        }}

        .page-btn {{
            padding: 0.5rem 1rem;
            background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.1);
            border-radius: 6px;
            color: #fff;
            cursor: pointer;
        }}

        .page-btn.active {{
            background: #667eea;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="search-header">
            <a href="/" class="back-link">← Back to Marketplace</a>

            <div class="search-box">
                <input type="text" id="searchInput" placeholder="Search plugins...">
            </div>

            <div class="filters">
                <div class="filter">
                    <select id="categoryFilter">
                        <option value="">All Categories</option>
                        <option value="tools">Tools</option>
                        <option value="integrations">Integrations</option>
                        <option value="themes">Themes</option>
                        <option value="language-support">Language Support</option>
                        <option value="productivity">Productivity</option>
                        <option value="development">Development</option>
                    </select>
                </div>
                <div class="filter">
                    <select id="sortFilter">
                        <option value="relevance">Relevance</option>
                        <option value="downloads">Most Downloads</option>
                        <option value="rating">Highest Rated</option>
                        <option value="newest">Newest</option>
                        <option value="updated">Recently Updated</option>
                    </select>
                </div>
            </div>

            <div class="results-count" id="resultsCount">Loading...</div>
        </div>

        <div class="plugins-grid" id="results">
            <!-- Results will be loaded here -->
        </div>

        <div class="pagination" id="pagination">
            <!-- Pagination will be loaded here -->
        </div>
    </div>

    <script>
        let currentPage = 1;
        let currentQuery = '';

        async function searchPlugins(page = 1) {{
            const query = document.getElementById('searchInput').value;
            const category = document.getElementById('categoryFilter').value;
            const sort = document.getElementById('sortFilter').value;

            currentQuery = query;
            currentPage = page;

            try {{
                let url = `/api/plugins?page=${{page}}`;
                if (query) url += `&q=${{encodeURIComponent(query)}}`;
                if (category) url += `&category=${{category}}`;
                if (sort) url += `&sort=${{sort}}`;

                const response = await fetch(url);
                const data = await response.json();

                if (data.success) {{
                    renderResults(data.data);
                }}
            }} catch (error) {{
                console.error('Search failed:', error);
            }}
        }}

        function renderResults(data) {{
            document.getElementById('resultsCount').textContent =
                `${{data.total}} plugin${{data.total !== 1 ? 's' : ''}} found`;

            const container = document.getElementById('results');
            container.innerHTML = data.plugins.map(plugin => `
                <div class="plugin-card">
                    <div class="plugin-header">
                        <div class="plugin-icon">🔌</div>
                        <div class="plugin-info">
                            <div class="plugin-name">${{plugin.name}}</div>
                            <div class="plugin-author">by ${{plugin.author}}</div>
                        </div>
                    </div>
                    <div class="plugin-description">${{plugin.description}}</div>
                    <div class="plugin-footer">
                        <div class="plugin-stats">
                            <span>⬇️ ${{formatNumber(plugin.downloads)}}</span>
                            <span>⭐ ${{plugin.rating}}</span>
                        </div>
                        <a href="/plugin/${{plugin.id}}" style="color: #667eea; text-decoration: none;">Details →</a>
                    </div>
                </div>
            `).join('');

            // Render pagination
            renderPagination(data);
        }}

        function renderPagination(data) {{
            const container = document.getElementById('pagination');
            let html = '';

            for (let i = 1; i <= data.total_pages; i++) {{
                html += `<button class="page-btn ${{i === data.page ? 'active' : ''}}" onclick="searchPlugins(${{i}})">${{i}}</button>`;
            }}

            container.innerHTML = html;
        }}

        function formatNumber(num) {{
            if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M';
            if (num >= 1000) return (num / 1000).toFixed(1) + 'K';
            return num.toString();
        }}

        // Event listeners
        document.getElementById('searchInput').addEventListener('keypress', function(e) {{
            if (e.key === 'Enter') searchPlugins(1);
        }});

        document.getElementById('categoryFilter').addEventListener('change', () => searchPlugins(1));
        document.getElementById('sortFilter').addEventListener('change', () => searchPlugins(1));

        // Initial search
        searchPlugins();
    </script>
</body>
</html>"#
    )
}
