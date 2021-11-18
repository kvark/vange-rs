#!/bin/bash
set -e
rm -rf assets/res && cp -r ../res assets/res
NODE_ENV=production npx tailwindcss -i road.in.css -c ./tailwind.config.js -o ./assets/road.css