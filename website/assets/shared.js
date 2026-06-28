/* ============================================================
   WechatAgent 官网 — 共享脚本层
   双语引擎 + 滚动入场动画 + 导航交互 + 数字滚动
   零依赖 vanilla JS
   ============================================================ */
(function () {
  "use strict";

  /* ---------- 双语引擎 ---------- */
  var STORAGE_KEY = "weagent-lang";
  function getLang() {
    return localStorage.getItem(STORAGE_KEY) || "zh";
  }
  function setLang(lang) {
    localStorage.setItem(STORAGE_KEY, lang);
    document.documentElement.setAttribute("lang", lang === "en" ? "en" : "zh");
    document.querySelectorAll("[data-lang-btn]").forEach(function (b) {
      b.classList.toggle("active", b.getAttribute("data-lang-btn") === lang);
    });
    // 更新 document.title
    var t = document.querySelector("title");
    if (t) {
      var zh = t.getAttribute("data-zh"), en = t.getAttribute("data-en");
      if (zh && en) t.textContent = lang === "en" ? en : zh;
    }
  }
  window.WeAgentLang = { get: getLang, set: setLang };

  /* ---------- 导航交互：滚动加深 + 移动菜单 ---------- */
  function initNav() {
    var nav = document.querySelector(".nav");
    if (nav) {
      var onScroll = function () {
        nav.classList.toggle("nav-solid", window.scrollY > 24);
      };
      onScroll();
      window.addEventListener("scroll", onScroll, { passive: true });
    }
    var burger = document.querySelector(".nav-burger");
    var menu = document.querySelector(".mobile-menu");
    if (burger && menu) {
      burger.addEventListener("click", function () {
        menu.classList.toggle("open");
      });
      menu.querySelectorAll("a").forEach(function (a) {
        a.addEventListener("click", function () { menu.classList.remove("open"); });
      });
    }
  }

  /* ---------- 滚动入场动画 ---------- */
  function initReveal() {
    var els = document.querySelectorAll(".reveal");
    if (!("IntersectionObserver" in window) || !els.length) {
      els.forEach(function (e) { e.classList.add("in"); });
      return;
    }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add("in");
          io.unobserve(entry.target);
        }
      });
    }, { threshold: 0.12, rootMargin: "0px 0px -8% 0px" });
    els.forEach(function (e) { io.observe(e); });
  }

  /* ---------- 数字滚动计数 ---------- */
  function animateCount(el) {
    var target = parseFloat(el.getAttribute("data-count"));
    var suffix = el.getAttribute("data-suffix") || "";
    var prefix = el.getAttribute("data-prefix") || "";
    var decimals = (el.getAttribute("data-decimals") | 0);
    var dur = 1400, start = null;
    function step(ts) {
      if (!start) start = ts;
      var p = Math.min((ts - start) / dur, 1);
      var eased = 1 - Math.pow(1 - p, 3);
      var val = (target * eased).toFixed(decimals);
      el.textContent = prefix + val + suffix;
      if (p < 1) requestAnimationFrame(step);
      else el.textContent = prefix + target.toFixed(decimals) + suffix;
    }
    requestAnimationFrame(step);
  }
  function initCounters() {
    var els = document.querySelectorAll("[data-count]");
    if (!("IntersectionObserver" in window) || !els.length) {
      els.forEach(function (e) {
        var t = parseFloat(e.getAttribute("data-count"));
        e.textContent = (e.getAttribute("data-prefix") || "") + t + (e.getAttribute("data-suffix") || "");
      });
      return;
    }
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) { animateCount(entry.target); io.unobserve(entry.target); }
      });
    }, { threshold: 0.5 });
    els.forEach(function (e) { io.observe(e); });
  }

  /* ---------- 微信号点击复制 + Toast ---------- */
  var toastTimer = null;
  function showToast(msg) {
    var el = document.querySelector(".toast");
    if (!el) {
      el = document.createElement("div");
      el.className = "toast";
      el.setAttribute("role", "status");
      document.body.appendChild(el);
    }
    el.innerHTML =
      '<svg viewBox="0 0 24 24" fill="none"><path d="M20 6L9 17l-5-5" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>' +
      '<span></span>';
    el.querySelector("span").textContent = msg;
    requestAnimationFrame(function () { el.classList.add("show"); });
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { el.classList.remove("show"); }, 2200);
  }
  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed"; ta.style.left = "-9999px";
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand("copy") ? resolve() : reject(); }
      catch (e) { reject(e); }
      document.body.removeChild(ta);
    });
  }
  function initCopy() {
    document.querySelectorAll("[data-wx]").forEach(function (el) {
      el.addEventListener("click", function () {
        var id = el.getAttribute("data-wx");
        var en = getLang() === "en";
        copyText(id).then(
          function () { showToast(en ? "WeChat ID copied: " + id : "已复制微信号：" + id); },
          function () { showToast(en ? "Please copy manually: " + id : "请手动复制：" + id); }
        );
      });
    });
  }

  /* ---------- 启动 ---------- */
  document.addEventListener("DOMContentLoaded", function () {
    setLang(getLang());
    document.querySelectorAll("[data-lang-btn]").forEach(function (b) {
      b.addEventListener("click", function () { setLang(b.getAttribute("data-lang-btn")); });
    });
    initNav();
    initReveal();
    initCounters();
    initCopy();
  });
})();
