# TV Launcher Media-Server + Musik-Server: plain Node, no npm packages -> tiny image.
# The film server is the default command; docker-compose.yml starts the music
# server from the same image with `node musicserver.js`.
FROM node:20-alpine
WORKDIR /app
# ffmpeg: plays MKV/AVI/TS films (live remux to MP4, see videotools.js).
RUN apk add --no-cache ffmpeg
COPY mediaserver.js musicserver.js videotools.js ./
ENV MEDIA_DIR=/media MUSIC_DIR=/music DATA_DIR=/data PORT=8420
EXPOSE 8420 8421
USER node
CMD ["node", "mediaserver.js"]
