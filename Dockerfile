FROM rust:latest

COPY . .

RUN cargo install --path .

ENV KITTYKAT_LISTEN_ADDRESS="0.0.0.0:1234"

EXPOSE 1234/tcp

CMD [ "kittykat" ]
