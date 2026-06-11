.PHONY: build server adminsrv adminctl test tidy run-server run-adminsrv migrate clean

build: server adminsrv adminctl

server:
	go build -o bin/server ./cmd/server

adminsrv:
	go build -o bin/adminsrv ./cmd/adminsrv

adminctl:
	go build -o bin/adminctl ./cmd/adminctl

test:
	go test ./...

tidy:
	go mod tidy

run-server: server
	./bin/server

run-adminsrv: adminsrv
	./bin/adminsrv

# 迁移到最新（首次初始化）
migrate: adminctl
	./bin/adminctl migrate up

clean:
	rm -rf bin
