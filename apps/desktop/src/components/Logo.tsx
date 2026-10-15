/** Nightwing mark uses the current theme's foreground and accent. */
export function Logo({ size = 32, className = "" }: { size?: number; className?: string }) {
  return <svg width={size} height={size} className={`logo ${className}`} viewBox="0 0 256 256" role="img" aria-label="CrowDM">
    <g fill="var(--logo-ink, var(--text))">
      <path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z" />
      <path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z" transform="translate(256 0) scale(-1 1)" />
      <path fillRule="evenodd" d="M112 96V72L102 78 108 59C112 40 124 30 140 30H153L147 38 162 45C179 47 188 57 186 76L177 68 165 65H156L144 80V96ZM135 47 155 53 142 59Z" />
      <path fill="var(--logo-accent, var(--accent))" d="M112 104H144V164H168L128 204 88 164H112Z" />
      <path d="M76 204H88V216H168V204H180V228H76Z" />
    </g>
  </svg>;
}
