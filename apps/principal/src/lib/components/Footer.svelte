<script lang="ts">
  // Footer is small + quiet — version, links, no dazzle.
</script>

<footer>
  <hr />
  <div class="row">
    <span class="dim">Paschal — a digital letter of last resort.</span>
    <ul>
      <li><a href="https://github.com/" rel="external noopener">Source</a></li>
    </ul>
  </div>
</footer>

<style>
  footer {
    max-width: 960px;
    margin: 0 auto;
    padding: 0 var(--sp-3) var(--sp-4);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--size-caption);
    color: var(--slate);
  }
  ul { list-style: none; padding: 0; margin: 0; display: flex; gap: var(--sp-3); }
  ul a { color: var(--slate); text-decoration: none; }
  ul a:hover { color: var(--ink); text-decoration: underline; text-underline-offset: 4px; }
</style>
