# Hablog
This project is a simple TCP proxy implemented in Rust using the Tokio asynchronous runtime. 
It allows forwarding network traffic between a client and a server. It logs the packets (kinda... buffer is wrong so message sizes are all over the place I think) sent between the client and server.
This was kinda just an excuse to learn more Rust, but it'd be pretty easy to use as a starting point to add packet headers / packet sending.

## Project Structure
The project consists of the following files:

main.rs: This file contains the main entry point of the application. It sets up the proxy server, resolves the host, and starts the connection.

connection.rs: This file contains the implementation of the Connection struct and related functions. The Connection struct represents the proxy connection and holds information about the connection state, host, port, and IP address. It also contains methods for resolving the host, starting the connection, handling incoming connections, and forwarding data between the client and server.

### You will need root privileges to run this.
The application checks the /etc/hosts file for a proxy entry for the specified host. If the entry is found, it displays a message. If the entry is not found, it adds a proxy entry for the host in the /etc/hosts file.





https://github.com/Libaration/Hablog/assets/11550216/e9964a4a-b530-4934-813f-c4478fc4e4dc

