FROM alpine:3.19
COPY entrypoint.sh pre.sh post.sh /
RUN chmod +x /entrypoint.sh /pre.sh /post.sh
ENTRYPOINT ["/entrypoint.sh"]
