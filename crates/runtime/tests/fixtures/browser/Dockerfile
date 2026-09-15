FROM python:3.12-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libglib2.0-0 libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libdbus-1-3 \
    libx11-6 libxcomposite1 libxdamage1 libxext6 libxfixes3 libxrandr2 libgbm1 \
    libexpat1 libxcb1 libxkbcommon0 libasound2 fonts-liberation \
    && rm -rf /var/lib/apt/lists/*
COPY chrome /opt/chromium
RUN ln -s /opt/chromium/chrome-headless-shell /usr/bin/chromium
