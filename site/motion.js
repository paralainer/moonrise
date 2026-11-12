// The page's two moving pictures: the film at the top and the recording of the app. Both are muted
// and loop; each has a Pause button. The film plays at once, the recording when it scrolls into
// view (and stops when it leaves). With reduced motion neither plays by itself: each shows its
// still frame with the video's own controls. Nothing here talks to the network.
(function () {
  var still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var buttons = document.querySelectorAll('button[data-for]');
  Array.prototype.forEach.call(buttons, function (b) {
    var v = document.getElementById(b.getAttribute('data-for'));
    if (!v) return;
    if (still) { v.controls = true; return; }
    var userPaused = false;
    b.hidden = false;
    function label() { b.textContent = v.paused ? 'Play' : 'Pause'; }
    function play() { var p = v.play(); if (p && p.catch) p.catch(function () { v.controls = true; b.hidden = true; }); }
    v.addEventListener('play', label);
    v.addEventListener('pause', label);
    b.addEventListener('click', function () {
      if (v.paused) { userPaused = false; play(); } else { userPaused = true; v.pause(); }
    });
    if (v.id === 'film' || !('IntersectionObserver' in window)) { play(); return; }
    v.preload = 'auto';
    new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { if (!userPaused) play(); } else if (!v.paused) { v.pause(); }
      });
    }, { threshold: 0.4 }).observe(v);
  });
})();
