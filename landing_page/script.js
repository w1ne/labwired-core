/*
    Landed by HTML5 UP
    html5up.net | @ajlkn
    Free for personal and commercial use under the CCA 3.0 license (html5up.net/license)
*/

document.addEventListener('DOMContentLoaded', function () {

    // Scrolly links
    const scrollyLinks = document.querySelectorAll('.scrolly');
    scrollyLinks.forEach(link => {
        link.addEventListener('click', function (e) {
            e.preventDefault();
            const targetId = this.getAttribute('href').substring(1);
            const targetElement = document.getElementById(targetId);
            if (targetElement) {
                targetElement.scrollIntoView({ behavior: 'smooth' });
            }
        });
    });

    // Parallax Effect (Simple implementation)
    const banner = document.getElementById('banner');
    window.addEventListener('scroll', function () {
        if (window.innerWidth > 980) {
            const scrollValues = window.scrollY;
            if (banner) {
                banner.style.backgroundPosition = `center ${scrollValues * 0.5}px`;
            }
        } else {
            if (banner) banner.style.backgroundPosition = 'center center';
        }
    });

    // Nav Panel (Mobile) - Simplified toggle logic
    // Implementation of a full slide-out panel would require more complex DOM manipulation
    // For this task, we will ensure responsive stack layout via CSS media queries.

    // Header Scroll State
    const header = document.getElementById('header');
    window.addEventListener('scroll', function () {
        if (window.scrollY > 0) {
            header.classList.add('alt'); // Define .alt in CSS if needed for semi-transparency variance
        } else {
            header.classList.remove('alt');
        }
    });

    // Form handling
    const form = document.querySelector('form.cta');
    if (form) {
        form.addEventListener('submit', function (e) {
            e.preventDefault();
            const btn = form.querySelector('input[type="submit"]');
            const originalVal = btn.value;
            btn.value = 'Thanks for subscribing!';
            btn.style.backgroundColor = '#4caf50'; // Green

            setTimeout(() => {
                btn.value = originalVal;
                btn.style.backgroundColor = '';
                form.reset();
            }, 3000);
        });
    }

});
