// Sidebar: mark active link and scroll into view
document.addEventListener('DOMContentLoaded', function() {
    var path = location.pathname;
    var links = document.querySelectorAll('.sidebar a, .sidebar-nav a, aside a');
    links.forEach(function(a) {
        if (a.pathname === path || (path.endsWith('/') && a.pathname === path.slice(0, -1))) {
            a.classList.add('active');
            var nav = a.closest('.sidebar, .sidebar-nav, aside');
            if (nav) nav.scrollTop = a.offsetTop - nav.offsetHeight / 2;
        }
    });
});
