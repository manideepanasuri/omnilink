/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './ui/**/*.{svelte,js,ts,jsx,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        omni: {
          bg: '#0d1117',
          surface: '#161b22',
          surface2: '#21262d',
          border: '#30363d',
          borderLight: '#484f58',
          text: '#e6edf3',
          muted: '#8b949e',
          cyan: '#22d3ee',
          emerald: '#34d399',
        },
      },
    },
  },
  plugins: [],
};
