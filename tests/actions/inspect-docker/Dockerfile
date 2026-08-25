FROM alpine:3.19
COPY inspect.sh /inspect.sh
COPY unused.sh /unused.sh
RUN chmod +x /inspect.sh /unused.sh
ENTRYPOINT ["/unused.sh"]
